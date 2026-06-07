//! `writeonce-greeter` — the TUI login greeter for tty1.
//!
//! Loop (mirrors writeonce-login, plus a session selector and the PAM env
//! hints that make logind treat the session as graphical):
//!   1. render banner; read username (default `writeonce`).
//!   2. look up the user; build the session menu (installed Wayland sessions
//!      discovered from the Nix profile + `/usr/share/wayland-sessions`, plus a
//!      synthetic "Shell (bash)"); pick one (remembered default).
//!   3. `pam_start("writeonce-greeter")`; set PAM_TTY + putenv XDG_SEAT/VTNR/
//!      SESSION_TYPE/CLASS *before* authenticate; authenticate → acct_mgmt →
//!      setcred(ESTABLISH) → open_session (pam_systemd → logind session +
//!      XDG_RUNTIME_DIR).
//!   4. `pam_getenvlist()`; fork; child drops privileges, builds env, and
//!      execs the chosen session (a compositor via `bash -lc 'exec …'`, or the
//!      login shell). Parent waits, closes the session, loops.
//!
//! It needs no DRM/seat of its own (it just writes text to the VT); the
//! compositor takes DRM master via logind once it starts. On any recoverable
//! error the loop re-renders rather than exiting — the unit also Restart=always.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::process;

use writeonce_greeter::config::Config;
use writeonce_greeter::sessions::{self, Session, SessionKind};
use writeonce_login::{pam, session as priv_session, term};

const DEFAULT_USER: &str = "writeonce";

#[derive(Debug)]
struct Args {
    tty: String,
    config: String,
}

fn parse_args() -> Args {
    let mut tty = "/dev/tty1".to_string();
    let mut config = "/etc/writeonce/greeter.toml".to_string();
    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--tty" => {
                i += 1;
                if i >= argv.len() {
                    die("--tty requires a value");
                }
                tty = argv[i].clone();
            }
            "--config" => {
                i += 1;
                if i >= argv.len() {
                    die("--config requires a value");
                }
                config = argv[i].clone();
            }
            "-h" | "--help" => {
                println!("Usage: writeonce-greeter [--tty PATH] [--config PATH]");
                process::exit(0);
            }
            other => die(&format!("unknown argument: {other}")),
        }
        i += 1;
    }
    Args { tty, config }
}

fn die(msg: &str) -> ! {
    eprintln!("writeonce-greeter: {msg}");
    process::exit(2);
}

fn main() {
    if let Err(e) = run() {
        eprintln!("writeonce-greeter: fatal: {e}");
        process::exit(1);
    }
}

struct UserInfo {
    name: String,
    uid: u32,
    gid: u32,
    home: String,
    shell: String,
}

/// `getpwnam`, copying strings out before the static buffer is reused.
fn lookup_user(name: &str) -> Option<UserInfo> {
    let c = CString::new(name).ok()?;
    let pw = unsafe { libc::getpwnam(c.as_ptr()) };
    if pw.is_null() {
        return None;
    }
    let pw = unsafe { *pw };
    let home = unsafe { CStr::from_ptr(pw.pw_dir) }.to_string_lossy().into_owned();
    let mut shell = unsafe { CStr::from_ptr(pw.pw_shell) }
        .to_string_lossy()
        .into_owned();
    if shell.is_empty() {
        shell = "/bin/bash".to_string();
    }
    Some(UserInfo {
        name: name.to_string(),
        uid: pw.pw_uid,
        gid: pw.pw_gid,
        home,
        shell,
    })
}

fn tty_name(tty_path: &str) -> String {
    tty_path.strip_prefix("/dev/").unwrap_or(tty_path).to_string()
}

fn vtnr_from(tty_path: &str) -> u32 {
    tty_path
        .strip_prefix("/dev/tty")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn run() -> io::Result<()> {
    let args = parse_args();
    let cfg = Config::load_or_default(&args.config);

    let tty_in = File::options().read(true).open(&args.tty)?;
    let mut tty_out = File::options().write(true).open(&args.tty)?;
    let vtnr = vtnr_from(&args.tty);
    let ttyname = tty_name(&args.tty);

    loop {
        let _ = render_banner(&mut tty_out, &cfg);

        // 1. Username (echo on; empty = the default user).
        let username = match term::read_line(
            &format!("  login [{DEFAULT_USER}]: "),
            tty_in.try_clone()?,
            &mut tty_out,
        ) {
            Ok(u) if !u.trim().is_empty() => u.trim().to_string(),
            Ok(_) => DEFAULT_USER.to_string(),
            Err(_) => continue,
        };

        // 2. Build the session menu from the user's home (or "/" if unknown —
        //    auth will fail later, but we don't leak which users exist).
        let user = lookup_user(&username);
        let home = user.as_ref().map(|u| u.home.clone()).unwrap_or_else(|| "/".to_string());
        let shell = user
            .as_ref()
            .map(|u| u.shell.clone())
            .unwrap_or_else(|| "/bin/bash".to_string());
        let menu = sessions::menu_for(&home, &shell);
        let didx = default_index(&menu, &cfg);
        let choice = match choose_session(&menu, didx, tty_in.try_clone()?, &mut tty_out) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let chosen = menu[choice].clone();

        // 3. PAM transaction for the user.
        let conv = TtyConv {
            tty_in: tty_in.try_clone()?,
            tty_out: tty_out.try_clone()?,
        };
        let mut session = match pam::Session::start(&cfg.pam_service, Some(&username), Box::new(conv)) {
            Ok(s) => s,
            Err(e) => {
                let _ = writeln!(tty_out, "  PAM start failed: {e}");
                pause();
                continue;
            }
        };

        // Session-classification hints — MUST be set before open_session so
        // pam_systemd binds the logind session to seat0/VT and types it.
        let _ = session.set_item(pam::PAM_TTY, &ttyname);
        let _ = session.putenv("XDG_SEAT=seat0");
        let _ = session.putenv(&format!("XDG_VTNR={vtnr}"));
        let _ = session.putenv("XDG_SESSION_CLASS=user");
        let stype = match chosen.kind {
            SessionKind::Wayland => "wayland",
            SessionKind::Shell => "tty",
        };
        let _ = session.putenv(&format!("XDG_SESSION_TYPE={stype}"));
        if chosen.kind == SessionKind::Wayland {
            let primary = chosen.primary_desktop();
            if !primary.is_empty() {
                let _ = session.putenv(&format!("XDG_SESSION_DESKTOP={primary}"));
            }
        }

        if let Err(e) = session.authenticate() {
            let _ = writeln!(tty_out, "  Login incorrect");
            eprintln!("writeonce-greeter: {e}");
            pause();
            continue;
        }
        if let Err(e) = session.acct_mgmt() {
            let _ = writeln!(tty_out, "  Account check failed: {e}");
            pause();
            continue;
        }
        if let Err(e) = session.establish_cred() {
            let _ = writeln!(tty_out, "  Could not establish credentials: {e}");
            pause();
            continue;
        }
        if let Err(e) = session.open_session() {
            let _ = writeln!(tty_out, "  Could not open session: {e}");
            pause();
            continue;
        }

        // Auth passed → the user must resolve.
        let user = match user.or_else(|| lookup_user(&username)) {
            Some(u) => u,
            None => {
                let _ = writeln!(tty_out, "  internal: user vanished after auth");
                let _ = session.close_session();
                let _ = session.delete_cred();
                continue;
            }
        };

        let pam_env = session.getenvlist();
        if chosen.kind == SessionKind::Wayland
            && !pam_env.iter().any(|(k, _)| k == "XDG_RUNTIME_DIR")
        {
            let _ = writeln!(
                tty_out,
                "  warning: logind gave no XDG_RUNTIME_DIR (pam_systemd?) — the compositor may"
            );
            let _ = writeln!(tty_out, "  fail to start; the Shell session still works.");
            eprintln!("writeonce-greeter: pam_getenvlist missing XDG_RUNTIME_DIR");
        }

        cfg.write_last_session(&chosen.name);
        let _ = writeln!(tty_out, "  starting {} …", chosen.name);

        // 4. Fork + launch the chosen session as the user.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            let _ = writeln!(tty_out, "  fork failed: {}", io::Error::last_os_error());
            let _ = session.close_session();
            let _ = session.delete_cred();
            continue;
        }
        if pid == 0 {
            launch_child(&user, &chosen, &pam_env, vtnr); // never returns
        }

        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };

        let _ = session.close_session();
        let _ = session.delete_cred();
        // loop → re-render the greeter
    }
}

/// Index of the remembered (or configured) default session, else 0.
fn default_index(menu: &[Session], cfg: &Config) -> usize {
    let remembered = cfg.read_last_session();
    let want = remembered
        .as_deref()
        .or(if cfg.default_session.is_empty() {
            None
        } else {
            Some(cfg.default_session.as_str())
        });
    if let Some(w) = want {
        if let Some(i) = menu.iter().position(|s| s.name == w) {
            return i;
        }
    }
    0
}

fn render_banner<W: Write>(tty: &mut W, cfg: &Config) -> io::Result<()> {
    let host = cfg.effective_hostname();
    writeln!(tty)?;
    writeln!(tty, "  ┌─ {} ───────────────────────────", cfg.welcome)?;
    writeln!(tty, "  │  {host} — sign in")?;
    writeln!(tty, "  └─────────────────────────────────")?;
    Ok(())
}

/// Render the numbered session list and read a choice (empty = default).
fn choose_session<R: Read, W: Write>(
    menu: &[Session],
    default_idx: usize,
    tty_in: R,
    tty_out: &mut W,
) -> io::Result<usize> {
    writeln!(tty_out, "  Session (⚙):")?;
    for (i, s) in menu.iter().enumerate() {
        let marker = if i == default_idx { "›" } else { " " };
        writeln!(tty_out, "   {marker} {}) {}", i + 1, s.name)?;
    }
    let prompt = format!("  choose [1-{}, Enter = {}]: ", menu.len(), default_idx + 1);
    let line = term::read_line(&prompt, tty_in, tty_out)?;
    let line = line.trim();
    if line.is_empty() {
        return Ok(default_idx);
    }
    match line.parse::<usize>() {
        Ok(n) if (1..=menu.len()).contains(&n) => Ok(n - 1),
        _ => Ok(default_idx),
    }
}

fn pause() {
    std::thread::sleep(std::time::Duration::from_secs(3));
}

/// Post-fork child: drop privileges, build env, exec the chosen session.
fn launch_child(user: &UserInfo, sess: &Session, pam_env: &[(String, String)], vtnr: u32) -> ! {
    if let Err(e) = priv_session::drop_privileges(user.uid, user.gid, &user.name) {
        eprintln!("writeonce-greeter: drop_privileges: {e}");
        unsafe { libc::_exit(127) };
    }
    let _ = priv_session::chdir(&user.home);

    let mut env = base_env(user, sess, vtnr);
    // Merge logind/pam_systemd's session env (XDG_RUNTIME_DIR, XDG_SESSION_ID,
    // DBUS_SESSION_BUS_ADDRESS, …). Our explicit keys win on conflict.
    for (k, v) in pam_env {
        let prefix = format!("{k}=");
        if !env.iter().any(|e| e.starts_with(&prefix)) {
            env.push(format!("{k}={v}"));
        }
    }

    let (prog, argv) = match sess.kind {
        SessionKind::Shell => {
            // Login shell: argv[0] gets a leading '-'.
            let base = sess.exec.rsplit('/').next().unwrap_or("bash");
            (sess.exec.clone(), vec![format!("-{base}")])
        }
        SessionKind::Wayland => {
            // Launch via a login shell so /etc/profile.d/nix.sh puts the Nix
            // profile on PATH and a bare Exec (e.g. "sway") resolves from
            // ~/.nix-profile/bin.
            (
                user.shell.clone(),
                vec![
                    user.shell.clone(),
                    "-l".to_string(),
                    "-c".to_string(),
                    format!("exec {}", sess.exec),
                ],
            )
        }
    };

    let err = priv_session::exec(&prog, &argv, &env);
    eprintln!("writeonce-greeter: exec {prog}: {err}");
    unsafe { libc::_exit(127) };
}

fn base_env(user: &UserInfo, sess: &Session, vtnr: u32) -> Vec<String> {
    let mut env = vec![
        format!("USER={}", user.name),
        format!("LOGNAME={}", user.name),
        format!("HOME={}", user.home),
        format!("SHELL={}", user.shell),
        "PATH=/usr/local/bin:/usr/bin:/bin".to_string(),
        "TERM=linux".to_string(),
        "LANG=C.UTF-8".to_string(),
        "XDG_SEAT=seat0".to_string(),
        format!("XDG_VTNR={vtnr}"),
        "XDG_SESSION_CLASS=user".to_string(),
    ];
    match sess.kind {
        SessionKind::Wayland => {
            env.push("XDG_SESSION_TYPE=wayland".to_string());
            let current = if sess.desktop_names.is_empty() {
                sess.name.clone()
            } else {
                sess.desktop_names.clone()
            };
            env.push(format!("XDG_CURRENT_DESKTOP={current}"));
            let primary = sess.primary_desktop();
            let primary = if primary.is_empty() { sess.name.as_str() } else { primary };
            env.push(format!("XDG_SESSION_DESKTOP={primary}"));
        }
        SessionKind::Shell => {
            env.push("XDG_SESSION_TYPE=tty".to_string());
        }
    }
    env
}

// ----------------------------------------------------------------------------
// Conversation — drives PAM prompts (the password) via the tty.
// ----------------------------------------------------------------------------

struct TtyConv {
    tty_in: File,
    tty_out: File,
}

impl pam::Conversation for TtyConv {
    fn prompt_echo_off(&mut self, msg: &str) -> Option<String> {
        let stdin = self.tty_in.try_clone().ok()?;
        term::read_password(&format!("  {msg}"), stdin, &mut self.tty_out).ok()
    }
    fn prompt_echo_on(&mut self, msg: &str) -> Option<String> {
        let stdin = self.tty_in.try_clone().ok()?;
        term::read_line(&format!("  {msg}"), stdin, &mut self.tty_out).ok()
    }
    fn info(&mut self, msg: &str) {
        let _ = writeln!(self.tty_out, "  {msg}");
    }
    fn error(&mut self, msg: &str) {
        let _ = writeln!(self.tty_out, "  [error] {msg}");
    }
}
