//! `writeonce-login` — PAM-based console login for tty1 (and friends).
//!
//! Loop:
//!   1. Render the banner + login prompt on the tty.
//!   2. Read username (echo on).
//!   3. Start a PAM transaction for the configured service.
//!   4. PAM raises a password prompt via our conversation callback;
//!      we read it from the tty with ECHO disabled.
//!   5. On `pam_authenticate` + `pam_acct_mgmt` success:
//!        - `pam_setcred(PAM_ESTABLISH_CRED)`
//!        - `pam_open_session()`
//!        - fork(); child drops privileges and execs the session script.
//!        - parent waits for the child, then `pam_close_session`,
//!          `pam_setcred(PAM_DELETE_CRED)`, and returns to step 1.
//!   6. On failure: print "Login incorrect", brief pause, loop.

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Write};
use std::process;

use writeonce_login::{config::Config, pam, term};

#[derive(Debug)]
struct Args {
    tty:    String,
    config: String,
}

fn parse_args() -> Args {
    let mut tty    = "/dev/tty1".to_string();
    let mut config = "/etc/writeonce/login.toml".to_string();

    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--tty" => {
                i += 1;
                if i >= argv.len() { die("--tty requires a value"); }
                tty = argv[i].clone();
            }
            "--config" => {
                i += 1;
                if i >= argv.len() { die("--config requires a value"); }
                config = argv[i].clone();
            }
            "-h" | "--help" => {
                println!("Usage: writeonce-login [--tty PATH] [--config PATH]");
                process::exit(0);
            }
            other => die(&format!("unknown argument: {other}")),
        }
        i += 1;
    }
    Args { tty, config }
}

fn die(msg: &str) -> ! {
    eprintln!("writeonce-login: {msg}");
    process::exit(2);
}

fn main() {
    if let Err(e) = run() {
        eprintln!("writeonce-login: fatal: {e}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args();
    let cfg = Config::load_or_default(&args.config);

    // Open the tty for read+write. Both stdin and stdout/stderr are
    // redirected by writeonce-svc when it spawns getty@ttyN.service via
    // dup2, so we don't strictly need to re-open; doing so anyway keeps
    // us robust when the binary is run by hand for testing.
    let tty_in  = File::options().read(true) .open(&args.tty)?;
    let mut tty_out = File::options().write(true).open(&args.tty)?;

    loop {
        render_banner(&mut tty_out, &cfg)?;

        // 1. Username (echo on).
        let username = term::read_line("login: ", &tty_in, &mut tty_out)?;
        if username.is_empty() {
            // Empty username; reprompt.
            writeln!(tty_out)?;
            continue;
        }

        // 2. PAM dance.
        let conv = TtyConv { tty_in: tty_in.try_clone()?, tty_out: tty_out.try_clone()? };
        let mut session = match pam::Session::start(&cfg.pam_service, Some(&username), Box::new(conv)) {
            Ok(s) => s,
            Err(e) => {
                writeln!(tty_out, "PAM start failed: {e}")?;
                pause();
                continue;
            }
        };

        if let Err(e) = session.authenticate() {
            writeln!(tty_out, "Login incorrect")?;
            eprintln!("writeonce-login: {e}");
            pause();
            continue;
        }
        if let Err(e) = session.acct_mgmt() {
            writeln!(tty_out, "Account validation failed: {e}")?;
            pause();
            continue;
        }
        if let Err(e) = session.establish_cred() {
            writeln!(tty_out, "Could not establish credentials: {e}")?;
            pause();
            continue;
        }
        if let Err(e) = session.open_session() {
            writeln!(tty_out, "Could not open session: {e}")?;
            pause();
            continue;
        }

        let resolved_user = session.authenticated_user().unwrap_or(username.clone());
        writeln!(tty_out, "writeonce-login: opening session for {resolved_user}")?;

        // 3. Fork + exec the session script.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            writeln!(tty_out, "fork failed: {}", io::Error::last_os_error())?;
            continue;
        }
        if pid == 0 {
            // Child branch — never returns on success.
            child_exec(&resolved_user, &cfg.session_script, &args.tty);
            // child_exec exits internally
        }

        // Parent: wait for the session to end.
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        writeln!(tty_out, "\nwriteonce-login: session ended (status={status})")?;

        let _ = session.close_session();
        let _ = session.delete_cred();
        // Session drops, pam_end runs.
    }
}

fn render_banner<W: Write>(tty: &mut W, cfg: &Config) -> io::Result<()> {
    let host = cfg.effective_hostname();
    writeln!(tty)?;
    writeln!(tty, "{}", cfg.welcome)?;
    writeln!(tty, "{host} tty login")?;
    Ok(())
}

fn pause() {
    // 3-second pause after auth failure, matching getty/`login(1)` UX.
    std::thread::sleep(std::time::Duration::from_secs(3));
}

// ----------------------------------------------------------------------------
// Conversation impl — drives PAM prompts via the tty.
// ----------------------------------------------------------------------------

struct TtyConv {
    tty_in:  File,
    tty_out: File,
}

impl pam::Conversation for TtyConv {
    fn prompt_echo_off(&mut self, msg: &str) -> Option<String> {
        // Password-style prompt; ECHO disabled.
        let stdin = self.tty_in.try_clone().ok()?;
        term::read_password(msg, stdin, &mut self.tty_out).ok()
    }

    fn prompt_echo_on(&mut self, msg: &str) -> Option<String> {
        // Visible prompt.
        let stdin = self.tty_in.try_clone().ok()?;
        term::read_line(msg, stdin, &mut self.tty_out).ok()
    }

    fn info(&mut self, msg: &str) {
        let _ = writeln!(self.tty_out, "{msg}");
    }

    fn error(&mut self, msg: &str) {
        let _ = writeln!(self.tty_out, "[error] {msg}");
    }
}

// ----------------------------------------------------------------------------
// Child: exec writeonce-session-create AS ROOT. It does the D-Bus
// CreateSession dance with writeonce-logind, then drops privileges
// itself and execve's the final session script. We deliberately do
// NOT drop privileges here — CreateSession needs root.
// ----------------------------------------------------------------------------

fn child_exec(user: &str, session_script: &str, tty_path: &str) -> ! {
    // Resolve uid/gid/home/shell up-front; pass them as args to the
    // helper so it doesn't have to re-do getpwnam.
    let username_c = CString::new(user).unwrap();
    let pwent = unsafe { libc::getpwnam(username_c.as_ptr()) };
    if pwent.is_null() {
        eprintln!("writeonce-login(child): unknown user {user}");
        unsafe { libc::_exit(127) };
    }
    let pw = unsafe { *pwent };
    let uid = pw.pw_uid;
    let gid = pw.pw_gid;
    let home_c = unsafe { std::ffi::CStr::from_ptr(pw.pw_dir) };
    let home = home_c.to_string_lossy().into_owned();
    let shell_c = unsafe { std::ffi::CStr::from_ptr(pw.pw_shell) };
    let shell = shell_c.to_string_lossy().into_owned();

    // Derive VT number from the tty path: /dev/tty1 → 1, /dev/tty7 → 7,
    // anything else → 0 (e.g. a serial console; logind treats vtnr=0 as
    // "not on a VT").
    let vtnr: u32 = tty_path
        .strip_prefix("/dev/tty")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Build argv for writeonce-session-create. It runs AS ROOT, calls
    // CreateSession via D-Bus, drops privileges to (uid, gid), and
    // execve's the session script.
    let prog = CString::new("/usr/sbin/writeonce-session-create").unwrap();
    let argv_owned: Vec<CString> = [
        "writeonce-session-create",
        "--user", user,
        "--uid", &uid.to_string(),
        "--gid", &gid.to_string(),
        "--home", &home,
        "--shell", &shell,
        "--tty", tty_path,
        "--vtnr", &vtnr.to_string(),
        "--session-script", session_script,
    ]
    .iter()
    .map(|s| CString::new(*s).unwrap())
    .collect();

    let argv_ptrs: Vec<*const libc::c_char> = argv_owned
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    // Minimal env for the helper itself (it builds the user env later).
    let env_owned: Vec<CString> = [
        "PATH=/usr/sbin:/usr/bin:/sbin:/bin",
        "RUST_LOG=info",
    ]
    .iter()
    .map(|s| CString::new(*s).unwrap())
    .collect();
    let env_ptrs: Vec<*const libc::c_char> = env_owned
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    unsafe { libc::execve(prog.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr()) };
    eprintln!(
        "writeonce-login(child): execve writeonce-session-create: {}",
        io::Error::last_os_error()
    );
    unsafe { libc::_exit(127) };
}

