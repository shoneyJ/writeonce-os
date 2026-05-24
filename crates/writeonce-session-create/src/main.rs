// writeonce-session-create — post-login helper.
//
// Flow:
//   1. Parse args (--user, --uid, --gid, --home, --shell, --tty,
//      --vtnr, --session-script).
//   2. Open the system D-Bus AS ROOT (writeonce-login execs us with
//      privileges intact).
//   3. Call org.freedesktop.login1.Manager.CreateSession(...) to
//      register a session with writeonce-logind. Receive back:
//        - session id (e.g. "c1")
//        - object path
//        - runtime path ("/run/user/<uid>")
//        - lifecycle FIFO file descriptor
//        - echoed uid/seat/vtnr
//        - existing-flag (true if already-created)
//   4. Clear FD_CLOEXEC on the lifecycle FD so it survives execve.
//   5. Create $XDG_RUNTIME_DIR (mkdir + chown user:user, mode 0700).
//   6. initgroups + setresgid + setresuid to the authenticated user.
//   7. chdir $HOME.
//   8. Build env (USER, HOME, SHELL, PATH, XDG_*, TERM, LANG, …).
//   9. execve the session script (typically /usr/bin/startx).
//
// The lifecycle FD is the magic glue: as long as the user-shell chain
// keeps it open, the session is alive; when the chain dies (logout,
// crash), the kernel closes all FDs, writeonce-logind sees EPOLLHUP
// on its read-end, and the session is auto-released.

use std::ffi::CString;
use std::os::fd::IntoRawFd;
use std::path::PathBuf;
use std::process;

use anyhow::{Context, Result};
use log::{error, info};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedFd, OwnedObjectPath, OwnedValue};

#[derive(Debug)]
struct Args {
    user: String,
    uid: u32,
    gid: u32,
    home: PathBuf,
    shell: PathBuf,
    tty: String,
    vtnr: u32,
    session_script: PathBuf,
}

fn parse_args() -> Result<Args> {
    let mut user = None;
    let mut uid = None;
    let mut gid = None;
    let mut home = None;
    let mut shell = None;
    let mut tty = "/dev/tty1".to_string();
    let mut vtnr: u32 = 1;
    let mut session_script = None;

    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        let take = |i: &mut usize, name: &str| -> Result<String> {
            *i += 1;
            argv.get(*i)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("{name} requires a value"))
        };
        match argv[i].as_str() {
            "--user"           => user           = Some(take(&mut i, "--user")?),
            "--uid"            => uid            = Some(take(&mut i, "--uid")?.parse()?),
            "--gid"            => gid            = Some(take(&mut i, "--gid")?.parse()?),
            "--home"           => home           = Some(PathBuf::from(take(&mut i, "--home")?)),
            "--shell"          => shell          = Some(PathBuf::from(take(&mut i, "--shell")?)),
            "--tty"            => tty            = take(&mut i, "--tty")?,
            "--vtnr"           => vtnr           = take(&mut i, "--vtnr")?.parse()?,
            "--session-script" => session_script = Some(PathBuf::from(take(&mut i, "--session-script")?)),
            "-h" | "--help" => {
                println!("Usage: writeonce-session-create \\");
                println!("    --user NAME --uid N --gid N --home PATH --shell PATH \\");
                println!("    --tty PATH --vtnr N --session-script PATH");
                process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
        i += 1;
    }

    Ok(Args {
        user: user.context("--user is required")?,
        uid: uid.context("--uid is required")?,
        gid: gid.context("--gid is required")?,
        home: home.context("--home is required")?,
        shell: shell.context("--shell is required")?,
        tty,
        vtnr,
        session_script: session_script.context("--session-script is required")?,
    })
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .format_timestamp(None)
    .init();

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("writeonce-session-create: {e}");
            process::exit(2);
        }
    };

    if let Err(e) = run(args) {
        error!("writeonce-session-create: {e:?}");
        process::exit(1);
    }
}

#[derive(Debug)]
struct SessionInfo {
    id: String,
    runtime_path: String,
    /// Owned FD — caller MUST consume into a raw fd before drop.
    fifo_fd: OwnedFd,
}

fn run(args: Args) -> Result<()> {
    let our_pid = process::id();
    info!(
        "creating session for {} (uid={} pid={} tty={} vtnr={})",
        args.user, args.uid, our_pid, args.tty, args.vtnr
    );

    // 1. Register session with writeonce-logind.
    let session = create_session(&args, our_pid)
        .context("Manager.CreateSession failed")?;
    info!(
        "session id={} runtime_path={}",
        session.id, session.runtime_path
    );

    // Extract everything we need before moving fifo_fd into the FD
    // dance (which consumes the OwnedFd).
    let session_id = session.id.clone();
    let runtime_path = session.runtime_path.clone();

    // 2. Make the lifecycle FD survive execve.
    //    zbus's OwnedFd wraps std::os::fd::OwnedFd; unwrap into the
    //    std type so we can call into_raw_fd() (releases ownership;
    //    the FD stays open across execve once we also clear CLOEXEC).
    let std_fd: std::os::fd::OwnedFd = session.fifo_fd.into();
    let fd: i32 = std_fd.into_raw_fd();
    clear_cloexec(fd).context("clear FD_CLOEXEC on lifecycle fd")?;
    info!("lifecycle fd {fd} ready (CLOEXEC cleared)");

    // 3. Create XDG_RUNTIME_DIR if not present.
    setup_runtime_dir(&runtime_path, args.uid, args.gid)
        .context("setup XDG_RUNTIME_DIR")?;

    // 4. Drop privileges to the authenticated user.
    drop_privileges(&args.user, args.uid, args.gid)
        .context("drop privileges")?;
    info!("dropped to uid={} gid={}", args.uid, args.gid);

    // 5. chdir to $HOME.
    let home_c = CString::new(args.home.as_os_str().as_encoded_bytes())?;
    if unsafe { libc::chdir(home_c.as_ptr()) } != 0 {
        anyhow::bail!(
            "chdir({}): {}",
            args.home.display(),
            std::io::Error::last_os_error()
        );
    }

    // 6. Build env + execve.
    let env = build_env(&args, &session_id, &runtime_path)?;
    let env_ptrs: Vec<*const libc::c_char> = env
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    let prog = CString::new(args.session_script.as_os_str().as_encoded_bytes())?;
    let argv: Vec<CString> = vec![prog.clone()];
    let argv_ptrs: Vec<*const libc::c_char> = argv
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    info!("execve {}", args.session_script.display());
    unsafe { libc::execve(prog.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr()) };
    anyhow::bail!(
        "execve {}: {}",
        args.session_script.display(),
        std::io::Error::last_os_error()
    );
}

// ============================================================================
// D-Bus call to org.freedesktop.login1.Manager.CreateSession
// ============================================================================

fn create_session(args: &Args, pid: u32) -> Result<SessionInfo> {
    let conn = Connection::system()
        .context("open system D-Bus — is dbus.service running?")?;

    // CreateSession argument tuple. Matches the writeonce-logind
    // Manager interface signature exactly (see crates/writeonce-logind
    // /src/manager.rs `fn create_session`).
    let body = (
        args.uid,                                          // uid
        pid,                                                // pid
        "writeonce-login".to_string(),                     // service
        "tty".to_string(),                                 // type
        "user".to_string(),                                // class
        "".to_string(),                                    // desktop
        "seat0".to_string(),                               // seat_id
        args.vtnr,                                          // vtnr
        args.tty.clone(),                                  // tty
        "".to_string(),                                    // display
        false,                                              // remote
        "".to_string(),                                    // remote_user
        "".to_string(),                                    // remote_host
        Vec::<(String, OwnedValue)>::new(),                // properties
    );

    let reply = conn
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "CreateSession",
            &body,
        )
        .context("D-Bus call CreateSession")?;

    type Reply = (
        String,           // id
        OwnedObjectPath,  // object path
        String,           // runtime_path
        OwnedFd,          // fifo_fd
        u32,              // uid echo
        String,           // seat
        u32,              // vtnr echo
        bool,             // existing
    );
    let (id, _path, runtime_path, fifo_fd, _uid_echo, _seat, _vtnr_echo, _existing): Reply =
        reply
            .body()
            .deserialize()
            .context("deserialize CreateSession reply")?;

    Ok(SessionInfo {
        id,
        runtime_path,
        fifo_fd,
    })
}

// ============================================================================
// FD lifecycle: clear CLOEXEC so the fd survives execve
// ============================================================================

fn clear_cloexec(fd: i32) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        anyhow::bail!("fcntl F_GETFD: {}", std::io::Error::last_os_error());
    }
    let new_flags = flags & !libc::FD_CLOEXEC;
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFD, new_flags) };
    if rc < 0 {
        anyhow::bail!("fcntl F_SETFD: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

// ============================================================================
// XDG_RUNTIME_DIR setup
// ============================================================================

fn setup_runtime_dir(path: &str, uid: u32, gid: u32) -> Result<()> {
    let path_c = CString::new(path).context("runtime path NUL byte")?;

    // mkdir is idempotent-ish: EEXIST is fine.
    let rc = unsafe { libc::mkdir(path_c.as_ptr(), 0o700) };
    if rc != 0 {
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            anyhow::bail!("mkdir {path}: {e}");
        }
    }

    // chown user:user
    if unsafe { libc::chown(path_c.as_ptr(), uid, gid) } != 0 {
        anyhow::bail!("chown {path}: {}", std::io::Error::last_os_error());
    }

    // chmod 0700 (in case it pre-existed with looser permissions)
    if unsafe { libc::chmod(path_c.as_ptr(), 0o700) } != 0 {
        anyhow::bail!("chmod {path}: {}", std::io::Error::last_os_error());
    }

    info!("{path} ready (mode 0700, owner {uid}:{gid})");
    Ok(())
}

// ============================================================================
// Privilege drop: initgroups + setresgid + setresuid
// ============================================================================

fn drop_privileges(user: &str, uid: u32, gid: u32) -> Result<()> {
    let user_c = CString::new(user).context("username NUL byte")?;

    // Supplementary groups from /etc/group.
    if unsafe { libc::initgroups(user_c.as_ptr(), gid) } != 0 {
        anyhow::bail!("initgroups: {}", std::io::Error::last_os_error());
    }

    // gid: setresgid before setresuid so we don't lose the ability to setgid.
    if unsafe { libc::setresgid(gid, gid, gid) } != 0 {
        anyhow::bail!("setresgid: {}", std::io::Error::last_os_error());
    }
    if unsafe { libc::setresuid(uid, uid, uid) } != 0 {
        anyhow::bail!("setresuid: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

// ============================================================================
// Environment for the execve'd session script
// ============================================================================

fn build_env(args: &Args, session_id: &str, runtime_path: &str) -> Result<Vec<CString>> {
    let path = "/run/current-system/sw/bin:/usr/local/bin:/usr/bin:/bin";
    let pairs = vec![
        format!("USER={}", args.user),
        format!("LOGNAME={}", args.user),
        format!("HOME={}", args.home.display()),
        format!("SHELL={}", args.shell.display()),
        format!("PATH={path}"),
        format!("XDG_SESSION_ID={session_id}"),
        format!("XDG_RUNTIME_DIR={runtime_path}"),
        "XDG_SESSION_CLASS=user".to_string(),
        "XDG_SESSION_TYPE=tty".to_string(),
        "TERM=linux".to_string(),
        "LANG=C.UTF-8".to_string(),
    ];
    pairs
        .into_iter()
        .map(|s| CString::new(s).context("env contains NUL byte"))
        .collect()
}

