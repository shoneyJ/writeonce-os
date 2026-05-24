// inhibitor.rs — lifecycle tracking for Manager.Inhibit() handles.
//
// systemd-logind's Inhibit() returns a file descriptor whose close
// ends the inhibition. We mirror that with a pipe: the daemon keeps
// the read-end, the caller gets the write-end. When the caller closes
// theirs (process exits, kernel closes all FDs), our read-end gets
// EPOLLHUP from the kernel and we drop the inhibitor record without
// the caller needing to call any "ReleaseInhibitor".
//
// The watcher runs on its own thread. It owns the epoll fd and an
// id→OwnedFd map; when EPOLLHUP fires, it grabs the AppState mutex
// briefly to remove the matching inhibitor, then closes the fd.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Arc, Mutex};
use std::thread;

use log::{error, info, warn};

use crate::state::AppState;

/// Add-this-fd request enqueued from the D-Bus method thread.
struct AddRequest {
    id: u32,
    fd: OwnedFd,
}

pub struct InhibitorWatcher {
    /// epoll fd. Kept open for the lifetime of the daemon.
    epoll_fd: OwnedFd,
    /// id → read-end of the pipe. Kept owned so the fd stays valid
    /// for epoll; dropped (closed) when the inhibitor is released.
    fds: Mutex<HashMap<u32, OwnedFd>>,
    /// Wake-up pipe so we can break out of epoll_wait when a new
    /// inhibitor is added. write-end here is woken by enqueue, the
    /// read-end is watched by epoll.
    wake_read: OwnedFd,
    wake_write: OwnedFd,
    /// Queue of new inhibitor registrations.
    pending: Mutex<Vec<AddRequest>>,
}

impl InhibitorWatcher {
    pub fn new() -> std::io::Result<Arc<Self>> {
        let epoll_fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if epoll_fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: epoll_create1 returned a valid fd.
        let epoll_fd = unsafe { OwnedFd::from_raw_fd_owned(epoll_fd) };

        let (wake_read, wake_write) = make_pipe_cloexec()?;
        let watcher = Arc::new(Self {
            epoll_fd,
            fds: Mutex::new(HashMap::new()),
            wake_read,
            wake_write,
            pending: Mutex::new(Vec::new()),
        });

        // Register the wake pipe so add() can interrupt epoll_wait.
        let mut ev = libc::epoll_event {
            events: libc::EPOLLIN as u32,
            u64: u64::MAX, // sentinel for "wake pipe", distinguished from any real id
        };
        let rc = unsafe {
            libc::epoll_ctl(
                watcher.epoll_fd.as_raw_fd(),
                libc::EPOLL_CTL_ADD,
                watcher.wake_read.as_raw_fd(),
                &mut ev,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(watcher)
    }

    /// Start the background thread. Called once at daemon startup.
    pub fn spawn(self: &Arc<Self>, app_state: Arc<Mutex<AppState>>) {
        let me = Arc::clone(self);
        thread::Builder::new()
            .name("inhibitor-watcher".into())
            .spawn(move || {
                me.run(app_state);
            })
            .expect("spawn inhibitor-watcher thread");
    }

    /// Enqueue a new inhibitor's read-fd for HUP watching. Wakes the
    /// watcher thread out of epoll_wait via the wake pipe.
    pub fn register(&self, id: u32, fd: OwnedFd) {
        self.pending.lock().unwrap().push(AddRequest { id, fd });
        // Write one byte to the wake pipe to break epoll_wait.
        let buf = [0u8; 1];
        let _ = unsafe {
            libc::write(self.wake_write.as_raw_fd(), buf.as_ptr() as *const _, 1)
        };
    }

    /// Main loop. Drains pending registrations + watches for HUPs.
    fn run(self: Arc<Self>, app_state: Arc<Mutex<AppState>>) {
        info!("inhibitor-watcher thread running");
        const MAX_EVENTS: usize = 32;
        let mut events: Vec<libc::epoll_event> = (0..MAX_EVENTS)
            .map(|_| libc::epoll_event { events: 0, u64: 0 })
            .collect();
        loop {
            // Drain pending adds before sleeping.
            self.drain_pending();

            let n = unsafe {
                libc::epoll_wait(
                    self.epoll_fd.as_raw_fd(),
                    events.as_mut_ptr(),
                    MAX_EVENTS as i32,
                    -1,
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                error!("epoll_wait failed: {err}");
                break;
            }
            for ev in &events[..n as usize] {
                if ev.u64 == u64::MAX {
                    // Wake pipe — drain it.
                    let mut buf = [0u8; 64];
                    let _ = unsafe {
                        libc::read(
                            self.wake_read.as_raw_fd(),
                            buf.as_mut_ptr() as *mut _,
                            buf.len(),
                        )
                    };
                    continue;
                }
                if (ev.events & (libc::EPOLLHUP as u32 | libc::EPOLLERR as u32)) != 0 {
                    let id = ev.u64 as u32;
                    self.release(id, &app_state);
                }
            }
        }
    }

    fn drain_pending(&self) {
        let pending = std::mem::take(&mut *self.pending.lock().unwrap());
        for req in pending {
            let mut ev = libc::epoll_event {
                events: (libc::EPOLLHUP | libc::EPOLLERR) as u32,
                u64: req.id as u64,
            };
            let rc = unsafe {
                libc::epoll_ctl(
                    self.epoll_fd.as_raw_fd(),
                    libc::EPOLL_CTL_ADD,
                    req.fd.as_raw_fd(),
                    &mut ev,
                )
            };
            if rc != 0 {
                warn!(
                    "epoll_ctl(ADD) for inhibitor {} failed: {}",
                    req.id,
                    std::io::Error::last_os_error()
                );
                // Don't store the fd if we couldn't watch it.
                continue;
            }
            self.fds.lock().unwrap().insert(req.id, req.fd);
        }
    }

    fn release(&self, id: u32, app_state: &Arc<Mutex<AppState>>) {
        // Remove from epoll first so we don't get re-fired.
        if let Some(fd) = self.fds.lock().unwrap().remove(&id) {
            let rc = unsafe {
                libc::epoll_ctl(
                    self.epoll_fd.as_raw_fd(),
                    libc::EPOLL_CTL_DEL,
                    fd.as_raw_fd(),
                    std::ptr::null_mut(),
                )
            };
            if rc != 0 {
                warn!(
                    "epoll_ctl(DEL) for inhibitor {id} failed: {}",
                    std::io::Error::last_os_error()
                );
            }
            // Dropping `fd` closes the read-end.
        }
        let mut st = app_state.lock().unwrap();
        st.inhibitors.retain(|i| i.id != id);
        info!("inhibitor {id} released (peer closed FD)");
    }
}

/// Create a CLOEXEC pipe and return (read, write) as OwnedFds.
pub fn make_pipe_cloexec() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1i32; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: pipe2(2) returned 0; both fds are valid.
    let read = unsafe { OwnedFd::from_raw_fd_owned(fds[0]) };
    let write = unsafe { OwnedFd::from_raw_fd_owned(fds[1]) };
    Ok((read, write))
}

// std::os::fd::OwnedFd::from_raw_fd is unsafe and lives in FromRawFd
// — the import noise is awkward enough to wrap here.
trait OwnedFdExt {
    /// SAFETY: caller must ensure the fd is a valid open file
    /// descriptor that they own (no other code will close it).
    unsafe fn from_raw_fd_owned(fd: i32) -> OwnedFd;
}

impl OwnedFdExt for OwnedFd {
    unsafe fn from_raw_fd_owned(fd: i32) -> OwnedFd {
        use std::os::fd::FromRawFd;
        OwnedFd::from_raw_fd(fd)
    }
}
