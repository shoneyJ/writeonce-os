//! WriteOnce OS — initramfs `/init` binary.
//!
//! Flow:
//!   1. Sanity check: process::id() == 1 (or `WO_INITRAMFS_FAKE=1`).
//!   2. Mount /proc, /sys, /dev (devtmpfs).
//!   3. Parse /proc/cmdline.
//!   4. Load kernel modules listed in /etc/modules-load.conf.
//!   5. Discover the root device (from `root=` in cmdline).
//!   6. switch_root + execve `/sbin/writeonce-pid1` (or `init=` override).
//!
//! On any error: drop to a minimal recovery shell that lets you inspect
//! /proc, /sys, /dev manually.

use std::io::{self, BufRead, Write};
use std::process;

use writeonce_initramfs::{cmdline::CmdLine, discover, modules, mount, switch_root};

fn main() {
    if let Err(e) = run() {
        eprintln!("writeonce-initramfs: fatal: {e}");
        recovery_shell();
        // Never returns.
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let fake = std::env::var("WO_INITRAMFS_FAKE").is_ok();
    if !fake && process::id() != 1 {
        return Err("not running as PID 1 (set WO_INITRAMFS_FAKE=1 to override)".into());
    }

    println!("writeonce-initramfs: starting (pid={}, fake={fake})", process::id());

    // 1. Mount essentials.
    if !fake {
        mount::essentials()?;
    } else {
        println!("writeonce-initramfs: WO_INITRAMFS_FAKE=1 — skipping mounts");
    }

    // 2. Parse cmdline.
    let cmd = CmdLine::load()?;
    println!("writeonce-initramfs: cmdline = {cmd:?}");

    if cmd.recovery {
        println!("writeonce-initramfs: wo.recovery set on cmdline — entering recovery shell");
        recovery_shell();
        // never returns
    }

    // 3. Load configured modules (best-effort; failures non-fatal).
    if !fake {
        modules::load_configured();
    }

    // 4. Discover the root device.
    let root_spec = cmd.root_spec.ok_or("no root= on /proc/cmdline")?;
    let root_dev = discover::locate_root(&root_spec, cmd.rootwait_secs)?;
    println!("writeonce-initramfs: root device = {}", root_dev.display());

    // 5. Mount + pivot + exec.
    let fstype = cmd.rootfstype.as_deref().unwrap_or("ext4");
    if !fake {
        switch_root::switch_and_exec(
            root_dev.to_str().ok_or("non-utf8 root path")?,
            fstype,
            cmd.mount_flags,
            &cmd.init_path,
        )?;
        // switch_and_exec returns `!` on success; if we got here it errored.
    } else {
        println!("writeonce-initramfs: WO_INITRAMFS_FAKE=1 — not pivoting; would execve {}", &cmd.init_path);
    }

    Ok(())
}

fn recovery_shell() -> ! {
    println!();
    println!("================================================================");
    println!("  WriteOnce OS — initramfs recovery shell");
    println!("================================================================");
    println!("  /proc, /sys, /dev are mounted (if PID 1).");
    println!("  Built-ins (no binaries on initramfs):");
    println!("    help                show this text");
    println!("    ls [PATH]           list directory  (default: /)");
    println!("    cat FILE            print file contents");
    println!("    cmdline             /proc/cmdline");
    println!("    mounts              /proc/mounts");
    println!("    blocks              /sys/class/block contents");
    println!("    exit                pause forever (PID 1 cannot exit cleanly)");
    println!("  Anything else is execvp'd in $PATH (rarely useful — no binaries).");
    println!();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        let _ = write!(stdout, "(recovery) # ");
        let _ = stdout.flush();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) | Err(_) => {
                // EOF or disconnected — pause forever rather than exit (PID 1).
                loop { unsafe { libc::pause() }; }
            }
            Ok(_) => {}
        }
        let line = line.trim();
        if line.is_empty() { continue; }
        if line == "exit" || line == "quit" {
            println!("writeonce-initramfs: pausing forever (PID 1 cannot exit cleanly)");
            loop { unsafe { libc::pause() }; }
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() { continue; }

        // Built-in commands — execute inside this Rust process so we
        // don't need binaries on the initramfs filesystem. These cover
        // every diagnostic a sysadmin reaches for in the first 30 sec
        // after dropping to a recovery shell.
        match parts[0] {
            "help" => {
                println!("  ls [PATH] | cat FILE | cmdline | mounts | blocks | exit");
                continue;
            }
            "ls" => {
                let path = parts.get(1).copied().unwrap_or("/");
                match std::fs::read_dir(path) {
                    Ok(entries) => {
                        for e in entries.flatten() {
                            let name = e.file_name();
                            let kind = e.file_type().ok().map(|t| {
                                if t.is_dir() { "d" }
                                else if t.is_symlink() { "l" }
                                else if t.is_file() { "f" }
                                else { "?" }
                            }).unwrap_or("?");
                            println!("  {kind}  {}", name.to_string_lossy());
                        }
                    }
                    Err(e) => println!("  ls: {path}: {e}"),
                }
                continue;
            }
            "cat" => {
                let Some(path) = parts.get(1) else { println!("  usage: cat FILE"); continue; };
                match std::fs::read_to_string(path) {
                    Ok(s) => print!("{s}{}", if s.ends_with('\n') { "" } else { "\n" }),
                    Err(e) => println!("  cat: {path}: {e}"),
                }
                continue;
            }
            "cmdline" => {
                let _ = std::fs::read_to_string("/proc/cmdline")
                    .map(|s| print!("{s}"))
                    .map_err(|e| println!("  /proc/cmdline: {e}"));
                continue;
            }
            "mounts" => {
                let _ = std::fs::read_to_string("/proc/mounts")
                    .map(|s| print!("{s}"))
                    .map_err(|e| println!("  /proc/mounts: {e}"));
                continue;
            }
            "blocks" => {
                match std::fs::read_dir("/sys/class/block") {
                    Ok(entries) => {
                        for e in entries.flatten() {
                            let name = e.file_name();
                            let sz_path = e.path().join("size");
                            let size = std::fs::read_to_string(&sz_path)
                                .ok()
                                .and_then(|s| s.trim().parse::<u64>().ok())
                                .map(|sectors| sectors * 512)
                                .map(|b| format!("{:.2} MiB", b as f64 / (1024.0*1024.0)))
                                .unwrap_or_else(|| "?".into());
                            println!("  /dev/{} — {size}", name.to_string_lossy());
                        }
                    }
                    Err(e) => println!("  /sys/class/block: {e}"),
                }
                continue;
            }
            _ => {}
        }

        // Tokenize on whitespace; exec the first as a program, the rest as argv.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            eprintln!("fork: {}", io::Error::last_os_error());
            continue;
        }
        if pid == 0 {
            use std::ffi::CString;
            let prog = match CString::new(parts[0]) { Ok(c) => c, Err(_) => unsafe { libc::_exit(127) } };
            let cs: Vec<CString> = parts.iter().map(|s| CString::new(*s).unwrap()).collect();
            let mut argv: Vec<*const libc::c_char> = cs.iter().map(|c| c.as_ptr()).collect();
            argv.push(std::ptr::null());
            unsafe { libc::execvp(prog.as_ptr(), argv.as_ptr()) };
            eprintln!("execvp {}: {}", parts[0], io::Error::last_os_error());
            unsafe { libc::_exit(127) };
        }
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
    }
}
