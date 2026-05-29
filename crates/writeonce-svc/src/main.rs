//! WriteOnce OS service supervisor — Phase-4 binary.
//!
//! Usage:
//!     writeonce-svc --units <dir> [--default-target <name>] [--fake]
//!
//! `--units <dir>` is the directory containing `*.service.toml` and
//! `*.target.toml` files (default `/etc/writeonce/services`).
//!
//! `--default-target` is the unit whose transitive closure becomes the
//! initial activation plan (default `default.target`).
//!
//! `--fake` skips `clone3(CLONE_INTO_CGROUP)` and uses plain `fork(2)`.
//! Useful on the workstation where `/sys/fs/cgroup/wo.slice/...` is
//! not writable.

use std::process;

use writeonce_svc::{config, control, enabled, signal, state::{self, SupervisorState}};

#[derive(Debug)]
struct Args {
    units_dir:      String,
    enabled_d:      String,
    default_target: String,
    socket:         String,
    log_dir:        String,
    fake:           bool,
}

fn parse_args() -> Args {
    let mut units_dir      = "/etc/writeonce/services".to_string();
    let mut enabled_d      = enabled::DEFAULT_DIR.to_string();
    let mut default_target = "default.target".to_string();
    let mut socket         = control::DEFAULT_SOCKET.to_string();
    let mut log_dir        = state::DEFAULT_LOG_DIR.to_string();
    let mut fake           = false;

    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--units" => {
                i += 1;
                if i >= argv.len() { die("--units requires a value"); }
                units_dir = argv[i].clone();
            }
            "--enabled-d" => {
                i += 1;
                if i >= argv.len() { die("--enabled-d requires a value"); }
                enabled_d = argv[i].clone();
            }
            "--default-target" => {
                i += 1;
                if i >= argv.len() { die("--default-target requires a value"); }
                default_target = argv[i].clone();
            }
            "--socket" => {
                i += 1;
                if i >= argv.len() { die("--socket requires a value"); }
                socket = argv[i].clone();
            }
            "--log-dir" => {
                i += 1;
                if i >= argv.len() { die("--log-dir requires a value"); }
                log_dir = argv[i].clone();
            }
            "--fake" => fake = true,
            "-h" | "--help" => {
                println!("Usage: writeonce-svc [--units DIR] [--enabled-d DIR] [--default-target NAME] [--socket PATH] [--log-dir DIR] [--fake]");
                process::exit(0);
            }
            other => die(&format!("unknown argument: {other}")),
        }
        i += 1;
    }

    Args { units_dir, enabled_d, default_target, socket, log_dir, fake }
}

fn die(msg: &str) -> ! {
    eprintln!("writeonce-svc: {msg}");
    process::exit(2);
}

fn main() {
    if let Err(e) = run() {
        eprintln!("writeonce-svc: fatal: {e}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args();

    println!("writeonce-svc: starting (units={}, enabled-d={}, default-target={}, log-dir={}, fake={})",
             args.units_dir, args.enabled_d, args.default_target, args.log_dir, args.fake);

    // 1. Load all unit files.
    let loaded = config::load_directory(&args.units_dir)?;
    println!("writeonce-svc: loaded {} unit(s)", loaded.len());
    for u in &loaded {
        println!("  - {}", u.name);
    }

    // 2. Build the supervisor state, then inject opt-in services from
    //    enabled.d/ as virtual `wanted-by = [multi-user.target]` edges.
    //    These only fire when multi-user.target is in the activation
    //    plan; default.target stops at console.target by default, so
    //    `wo-ctl enable` doesn't change boot behaviour unless the user
    //    also points default.target at multi-user.target.
    let mut state = SupervisorState::from_loaded(loaded, args.fake);
    state.enabled_d = args.enabled_d.clone();
    state.log_dir   = args.log_dir.clone();

    // Ensure the per-service log directory exists before we spawn anything.
    // Non-fatal: spawn() falls back to /dev/null per service if a log can't
    // be opened, so a read-only or missing path must not abort boot.
    if let Err(e) = std::fs::create_dir_all(&args.log_dir) {
        eprintln!("writeonce-svc: could not create log dir {}: {e} (service output → /dev/null)",
                  args.log_dir);
    }
    let enabled_units = enabled::load(&args.enabled_d)?;
    if !enabled_units.is_empty() {
        println!("writeonce-svc: enabled.d: {} unit(s)", enabled_units.len());
    }
    for unit_name in &enabled_units {
        match state.registry.add_wanted_by("multi-user.target", unit_name) {
            Ok(()) => println!("  + {unit_name}"),
            Err(e) => eprintln!("writeonce-svc: enabled.d: {e} (skipping)"),
        }
    }

    let plan = state.registry.build_transaction(&args.default_target)
        .map_err(|e| format!("transaction build failed: {e:?}"))?;
    println!("writeonce-svc: plan ({} job(s)):", plan.len());
    for job in &plan {
        println!("  -> {}  [{:?}]", state.registry.name_of(job.unit), job.kind);
    }

    // 3. Install the signal handler before spawning anything — children
    //    must see the supervisor with its mask already in place so the
    //    inherited mask is consistent.
    let signal_fd = signal::install()?;

    // 4. Bind the control-plane Unix socket.
    let listener = match control::ControlListener::bind(&args.socket) {
        Ok(l) => {
            println!("writeonce-svc: control socket at {}", &args.socket);
            Some(l)
        }
        Err(e) => {
            eprintln!("writeonce-svc: could not bind control socket {}: {} (continuing without)",
                      &args.socket, e);
            None
        }
    };

    // 5. Run the plan.
    state.activate_plan(&plan)?;
    state.print_summary();

    // 6. Block in the event loop until shutdown.
    println!("writeonce-svc: entering event loop");
    signal::event_loop(signal_fd, listener.as_ref(), &mut state)?;

    println!("writeonce-svc: clean shutdown");
    state.print_summary();
    Ok(())
}
