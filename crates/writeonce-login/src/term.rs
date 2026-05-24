//! Tiny tty helpers: print prompt, read line, read line with echo off.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, RawFd};

/// Read a line from `tty_in`, writing `prompt` to `tty_out` first.
pub fn read_line<R: Read, W: Write>(prompt: &str, tty_in: R, tty_out: &mut W) -> io::Result<String> {
    write!(tty_out, "{prompt}")?;
    tty_out.flush()?;
    let mut reader = BufReader::new(tty_in);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

/// Read a line from `tty_in` with terminal echo disabled (for passwords).
/// `tty_out` is used for the prompt and the trailing newline (echo-off
/// suppresses the newline the user types).
pub fn read_password<R: Read + AsRawFd, W: Write>(
    prompt:  &str,
    tty_in:  R,
    tty_out: &mut W,
) -> io::Result<String> {
    write!(tty_out, "{prompt}")?;
    tty_out.flush()?;

    let fd = tty_in.as_raw_fd();
    let guard = EchoGuard::install(fd)?;

    let mut reader = BufReader::new(tty_in);
    let mut line = String::new();
    let result = reader.read_line(&mut line);

    drop(guard);                          // restore termios
    writeln!(tty_out)?;                    // echo the suppressed newline

    let _ = result?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

/// RAII guard that disables ECHO on a tty fd for the prompt's lifetime
/// and restores the saved termios on drop.
struct EchoGuard {
    fd:    RawFd,
    saved: libc::termios,
}

impl EchoGuard {
    fn install(fd: RawFd) -> io::Result<Self> {
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut t = saved;
        t.c_lflag &= !libc::ECHO;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd, saved })
    }
}

impl Drop for EchoGuard {
    fn drop(&mut self) {
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved) };
    }
}
