//! Load kernel modules from the initramfs.
//!
//! Two sources of work:
//!   1. `/etc/modules-load.conf` (or the directory `/etc/modules-load.d/`):
//!      one module name per line, comments allowed.
//!   2. Every `*.ko` we find under `/lib/modules/`.
//!
//! We don't compute dependency order here — depmod metadata isn't part
//! of the minimal initramfs we care about. The supervisor's services
//! and the kernel will modprobe-on-demand for anything not pre-loaded.

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;

const MODULES_LOAD_CONF:  &str = "/etc/modules-load.conf";
const MODULES_LOAD_D_DIR: &str = "/etc/modules-load.d";

/// Returns the list of module names declared by config files, in order
/// (preserves the order they were listed in).
pub fn names_from_config() -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    if let Ok(body) = fs::read_to_string(MODULES_LOAD_CONF) {
        for line in body.lines() {
            extend_with_module_line(&mut names, line);
        }
    }
    if let Ok(entries) = fs::read_dir(MODULES_LOAD_D_DIR) {
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            if let Ok(body) = fs::read_to_string(&p) {
                for line in body.lines() {
                    extend_with_module_line(&mut names, line);
                }
            }
        }
    }
    Ok(names)
}

fn extend_with_module_line(out: &mut Vec<String>, line: &str) {
    let l = line.trim();
    if l.is_empty() || l.starts_with('#') { return; }
    out.push(l.to_string());
}

/// Try to load one module by *.ko file path. Uses `finit_module(2)`.
///
/// Errors are logged but non-fatal — a missing or unloadable module is
/// not enough reason to bring down the boot.
pub fn finit_module<P: AsRef<std::path::Path>>(path: P) -> io::Result<()> {
    let file = fs::File::open(&path)?;
    let fd = file.as_raw_fd();
    // SYS_finit_module(int fd, const char *param_values, int flags)
    let params = CString::new("").unwrap();
    let rc = unsafe {
        libc::syscall(libc::SYS_finit_module, fd, params.as_ptr(), 0)
    };
    if rc < 0 {
        let e = io::Error::last_os_error();
        // EEXIST means the module is already loaded — fine.
        if e.raw_os_error() == Some(libc::EEXIST) { return Ok(()); }
        return Err(e);
    }
    Ok(())
}

/// Find a `.ko` file matching the given module name under
/// `/lib/modules/`. Returns the first match (depmod ordering not
/// guaranteed in the initramfs context).
pub fn find_ko<P: AsRef<std::path::Path>>(root: P, name: &str) -> Option<std::path::PathBuf> {
    let want = format!("{name}.ko");
    let want_xz = format!("{name}.ko.xz");
    let want_zst = format!("{name}.ko.zst");
    walk(root.as_ref(), &|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n == want || n == want_xz || n == want_zst)
            .unwrap_or(false)
    })
}

fn walk<F: Fn(&std::path::Path) -> bool>(root: &std::path::Path, pred: &F) -> Option<std::path::PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = walk(&path, pred) {
                return Some(found);
            }
        } else if pred(&path) {
            return Some(path);
        }
    }
    None
}

/// Convenience: load every module listed in config files (best-effort).
pub fn load_configured() {
    let names = match names_from_config() {
        Ok(n) => n,
        Err(_) => return,
    };
    for name in names {
        let Some(path) = find_ko("/lib/modules", &name) else {
            eprintln!("writeonce-initramfs: module {name} not found");
            continue;
        };
        match finit_module(&path) {
            Ok(_)  => eprintln!("writeonce-initramfs: loaded {name}"),
            Err(e) => eprintln!("writeonce-initramfs: load {name}: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extend_skips_comments_and_blanks() {
        let mut v: Vec<String> = Vec::new();
        extend_with_module_line(&mut v, "");
        extend_with_module_line(&mut v, "   ");
        extend_with_module_line(&mut v, "# comment");
        extend_with_module_line(&mut v, "  i915 ");
        extend_with_module_line(&mut v, "ahci");
        assert_eq!(v, vec!["i915".to_string(), "ahci".to_string()]);
    }
}
