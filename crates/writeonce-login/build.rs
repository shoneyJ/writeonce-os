// Tell cargo to link against the system libpam at compile time.
//
// On the workstation this resolves to /usr/lib/x86_64-linux-gnu/libpam.so
// (installed via `apt install libpam0g-dev`). On the target sysroot
// (Phase 8) it resolves to $LFS/usr/lib/libpam.so. Either way, the
// resulting binary is dynamically linked.

fn main() {
    println!("cargo:rustc-link-lib=pam");
}
