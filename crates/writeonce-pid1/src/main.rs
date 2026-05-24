//! WriteOnce OS — PID 1 prototype.
//!
//! Implements the POSIX PID 1 contract on Linux:
//!   - mount /proc, /sys, /dev, /dev/pts, /run, /sys/fs/cgroup
//!   - block all signals and re-deliver them via signalfd
//!   - fork + execve a placeholder service (`/bin/sh` on tty1) on first boot
//!   - reap zombies via `waitpid(-1, WNOHANG)` on SIGCHLD
//!   - on SIGTERM/SIGINT, send SIGTERM to the child, wait `shutdown_grace`,
//!     escalate to SIGKILL, sync filesystems, `reboot(LINUX_REBOOT_CMD_RESTART)`
//!
//! For development on a non-PID-1 process set `WO_PID1_FAKE=1`; the binary
//! will then skip the PID-1 sanity check and the actual `mount(2)` calls.

mod config;
mod exec;
mod mount;
mod signal;

use std::process;

fn main() {
    if let Err(e) = run() {
        eprintln!("writeonce-pid1: fatal: {e}");
        // PID 1 exiting causes a kernel panic. Pause forever so netconsole
        // can capture the diagnostic instead.
        if process::id() == 1 {
            loop {
                unsafe { libc::pause() };
            }
        }
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let fake = std::env::var("WO_PID1_FAKE").is_ok();
    if !fake && process::id() != 1 {
        return Err("not running as PID 1 (set WO_PID1_FAKE=1 to override)".into());
    }

    println!(
        "writeonce-pid1: starting (pid={}, fake={fake})",
        process::id()
    );

    let cfg = config::Config::load_or_default();
    println!("writeonce-pid1: config: {cfg:?}");

    if !fake {
        mount::mount_essentials()?;
    } else {
        println!("writeonce-pid1: WO_PID1_FAKE=1 — skipping mount_essentials()");
    }

    let signal_fd = signal::install()?;
    let child_pid = exec::spawn_placeholder(&cfg)?;
    println!("writeonce-pid1: spawned child pid={child_pid}");

    signal::event_loop(signal_fd, child_pid, &cfg)
}
