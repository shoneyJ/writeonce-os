// writeonce-installer — read artifacts from Docker-driven build, write
// to a connected USB.
//
// CLI:
//   writeonce-installer list-usb
//   writeonce-installer install --from <artifacts-dir> --target /dev/sdX [--yes] [--dry-run]
//
// Always runs as root (needs sgdisk, mkfs, mount). Validates artifacts
// (SHA-256 vs manifest.toml) BEFORE any destructive operation. If the
// USB silently corrupts blocks during write, the post-write reread
// will catch it.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod bootloader;
mod confirm;
mod customize;
mod detect;
mod extract;
mod manifest;
mod mkfs;
mod mount_;
mod partition;
mod prompt;
mod spec;
mod tui;
mod verify;

#[derive(Parser)]
#[command(name = "writeonce-installer", about, version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// Verbose logging (RUST_LOG=debug equivalent).
    #[arg(long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show removable block devices that the installer would consider.
    ListUsb,

    /// Write a WriteOnce build to the chosen removable device.
    Install {
        /// Directory containing manifest.toml + artifacts.
        #[arg(long)]
        from: PathBuf,

        /// Block device to install onto, e.g. /dev/sdb. Pass nothing
        /// to interactively pick from a list.
        #[arg(long)]
        target: Option<PathBuf>,

        /// Optional target-os.json. Any field omitted/null in the file
        /// triggers an interactive prompt instead. Without --spec,
        /// every choice is prompted.
        #[arg(long)]
        spec: Option<PathBuf>,

        /// Skip the type-"yes" confirmation. Required for non-TTY use.
        #[arg(long)]
        yes: bool,

        /// Run all the steps that don't touch the disk; refuse before
        /// sgdisk. Used for CI / development.
        #[arg(long)]
        dry_run: bool,

        /// Override the safety check that refuses non-removable
        /// devices. Almost never what you want.
        #[arg(long)]
        force_non_removable: bool,

        /// Force the line-by-line CLI prompt flow even when stdin/stdout
        /// are TTYs. By default the installer launches a ratatui-based
        /// TUI; --no-tui falls back to plain prompts. Pair with --spec
        /// + --yes for fully non-interactive installs.
        #[arg(long)]
        no_tui: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let log_level = if cli.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level))
        .format_timestamp(None)
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        match cli.cmd {
            Cmd::ListUsb => list_usb(),
            Cmd::Install {
                from,
                target,
                spec,
                yes,
                dry_run,
                force_non_removable,
                no_tui,
            } => install(from, target, spec, yes, dry_run, force_non_removable, no_tui).await,
        }
    })
}

// ---------------------------------------------------------------------------
// list-usb
// ---------------------------------------------------------------------------

fn list_usb() -> Result<()> {
    let devices = detect::list_removable()?;
    if devices.is_empty() {
        println!("No removable block devices detected.");
        return Ok(());
    }
    println!("DEVICE      VENDOR             MODEL                    SIZE");
    for d in devices {
        println!(
            "{:<11} {:<18} {:<24} {:>6.1} GB",
            d.path.display(),
            d.vendor,
            d.model,
            d.size_gb()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

async fn install(
    from: PathBuf,
    target: Option<PathBuf>,
    spec_path: Option<PathBuf>,
    yes: bool,
    dry_run: bool,
    force_non_removable: bool,
    no_tui: bool,
) -> Result<()> {
    require_root()?;

    println!("[1/11] Loading manifest from {}", from.display());
    let (manifest, _path) = manifest::Manifest::load(&from)?;
    let artifacts = manifest.resolve(&from);
    log::info!("Manifest schema {} loaded", manifest.schema_version);

    println!("[2/11] Verifying source artifacts (SHA-256 vs manifest.toml) ...");
    artifacts.verify_against_manifest().await?;

    println!("[3/11] Loading target-os.json + gathering plan");
    let spec_obj = match spec_path.as_deref() {
        Some(p) => {
            println!("       Loading {}", p.display());
            spec::TargetOsSpec::load(p)?
        }
        None => spec::TargetOsSpec::default(),
    };

    // Choose the input flow:
    //   - --no-tui   → line-by-line CLI prompts (existing path)
    //   - --target X → operator already picked device; CLI prompts for the rest
    //   - --yes      → fully non-interactive; spec must be complete
    //   - else (TTY) → ratatui TUI
    let cli_path = no_tui || target.is_some() || yes || !is_tty();
    let (device, plan) = if cli_path {
        let device = select_target(target, force_non_removable)?;
        detect::safety_check(&device)
            .context("safety check on selected device")?;
        let plan = prompt::gather_plan(&spec_obj, &device)?;
        prompt::summarize(&device, &plan);
        (device, plan)
    } else {
        let mut devices = detect::list_removable()?;
        if devices.is_empty() {
            anyhow::bail!("no removable devices found; insert a USB and re-run");
        }
        // Apply safety_check inline so the TUI doesn't have to know about /proc/mounts.
        devices.retain(|d| {
            if let Err(e) = detect::safety_check(d) {
                log::warn!("skipping {}: {}", d.path.display(), e);
                false
            } else {
                true
            }
        });
        if devices.is_empty() {
            anyhow::bail!("no eligible removable devices (all currently mounted?)");
        }
        match tui::run_tui(&spec_obj, devices)? {
            Some(pair) => pair,
            None => anyhow::bail!("user cancelled the TUI; nothing written"),
        }
    };

    println!(
        "[4/11] Target {} — {} {} — {:.2} GB ({})",
        device.path.display(),
        device.vendor,
        device.model,
        device.size_gb(),
        if device.removable { "removable" } else { "FIXED" }
    );

    if dry_run {
        println!();
        println!("--dry-run set; would now wipe {} and continue.", device.path.display());
        println!("Stopping before any destructive operation.");
        return Ok(());
    }

    // TUI users already confirmed via the summary screen; CLI users
    // still get the type-"yes" prompt unless --yes.
    if !yes && cli_path && !confirm::confirm_wipe(&device)? {
        anyhow::bail!("user declined; nothing written");
    }

    println!("[5/11] Partitioning GPT (ESP {} MiB + root) ...", plan.partition.esp_mib);
    partition::create_gpt(&device.path, &plan.partition).await?;
    let (esp_part, root_part) = partition::partition_paths(&device.path);

    println!("[6/11] Formatting ESP + root");
    mkfs::format_esp(&esp_part).await?;
    let root_uuid = mkfs::format_root(&root_part).await?;

    println!("[7/11] Mounting target + extracting sysroot");
    let workdir = tempfile::tempdir().context("create workdir")?;
    let mount_root = workdir.path().join("root");
    mount_::mount(&root_part, &mount_root, "ext4").await?;
    let mount_guard_root = mount_::MountGuard::new(mount_root.clone());

    extract::extract_sysroot(&artifacts.sysroot, &mount_root).await?;

    println!("[8/11] Customising sysroot (user account + keyboard layout)");
    customize::apply(&plan, &mount_root)?;

    let esp_dir = mount_root.join("boot/efi");
    tokio::fs::create_dir_all(&esp_dir).await?;
    mount_::mount(&esp_part, &esp_dir, "vfat").await?;
    let mount_guard_esp = mount_::MountGuard::new(esp_dir.clone());

    println!("[9/11] Installing bootloader + kernel + initramfs to ESP");
    let cmdline = bootloader::format_cmdline(&artifacts.cmdline_template, &root_uuid);
    let layout = bootloader::EspLayout {
        esp_mount: &esp_dir,
        bootloader: &artifacts.bootloader,
        kernel: &artifacts.kernel,
        initramfs: &artifacts.initramfs,
        cmdline: &cmdline,
    };
    bootloader::populate_esp(&layout).await?;

    println!("[10/11] sync + unmount");
    sync_all().await;
    mount_::umount(&esp_dir).await?;
    mount_guard_esp.forget();
    mount_::umount(&mount_root).await?;
    mount_guard_root.forget();

    println!("[11/11] Verifying re-read (SHA-256)");
    verify::verify_post_write(&artifacts, &esp_part, workdir.path()).await?;

    println!();
    println!("✓ Install complete.");
    println!("  Eject {} and boot it on the target machine.", device.path.display());
    println!("  Cmdline: {cmdline}");
    println!("  Login as: {}", plan.user.name);
    Ok(())
}

fn select_target(
    target: Option<PathBuf>,
    force_non_removable: bool,
) -> Result<detect::UsbDevice> {
    if let Some(path) = target {
        let dev = detect::lookup(&path)
            .ok_or_else(|| anyhow::anyhow!("device {} not found under /sys/block", path.display()))?;
        if !dev.removable && !force_non_removable {
            anyhow::bail!(
                "{} is not removable. Pass --force-non-removable if you really mean to install \
                 onto an internal disk.",
                dev.path.display()
            );
        }
        return Ok(dev);
    }
    // No --target: enumerate + prompt to pick one.
    let devices = detect::list_removable()?;
    if devices.is_empty() {
        anyhow::bail!("no removable devices found; specify --target /dev/sdX explicitly");
    }
    if devices.len() == 1 {
        log::info!("only one removable device — selecting {}", devices[0].path.display());
        return Ok(devices[0].clone());
    }
    use std::io::{BufRead, Write};
    println!("Multiple removable devices detected:");
    for (i, d) in devices.iter().enumerate() {
        println!(
            "  [{i}] {} — {} {} ({:.2} GB)",
            d.path.display(),
            d.vendor,
            d.model,
            d.size_gb()
        );
    }
    print!("Select [0-{}]: ", devices.len() - 1);
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let idx: usize = line
        .trim()
        .parse()
        .context("expected a number")?;
    Ok(devices
        .get(idx)
        .ok_or_else(|| anyhow::anyhow!("index out of range"))?
        .clone())
}

fn require_root() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        anyhow::bail!("writeonce-installer must run as root (try `sudo`).");
    }
    Ok(())
}

async fn sync_all() {
    let _ = tokio::process::Command::new("sync").status().await;
}

fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}
