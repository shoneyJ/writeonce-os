# Fix libpam staging + dbus prerequisites (next boot blockers)

> Bring-up plan for the three failures visible in
> `.agents/PXL_20260527_213400915.jpg` — taken after the bare-minimum
> boot refit ([`fix-learn-from-scratch-boot.md`](fix-learn-from-scratch-boot.md))
> landed. Burst-cap is now correctly catching the spin (`hit
> start-limit-burst (3 failures in 30s)` lines visible in the photo),
> proving the meta-fix works. Three concrete service-startup issues
> remain. **Status: draft, awaiting user review.**

## Context

After the bare-minimum-boot refit, the supervisor reaches its plan
and starts the four console-target chain services. Three of them
fail with errors that the burst-cap correctly contains:

1. **`writeonce-login: error while loading shared libraries:
   libpam.so.0: cannot open shared object file or directory`**
2. **`dbus.service Failed (pid=… status=256)`** — exit code 1
3. **`writeonce-logind: Error: InputOutput(Os { code: 2, kind:
   NotFound, message: "No such file or directory" })`** — D-Bus
   connect fails because dbus isn't running

None of these is a regression — they all existed previously but
were hidden behind the "everything on fire" of the earlier
read-only-filesystem failure and the runaway respawn. With the
spin contained, they're individually tractable.

### Root causes (confirmed by audit)

| # | Symptom | Root cause | Cite |
|---|---------|------------|------|
| 1 | `libpam.so.0: cannot open shared object` | `libpam.so.0` is in `$LFS/lib64/` (where Phase 8a's linux-pam build installs it — wrong prefix); `17-stage-sysroot.sh` only copies `$LFS/usr/` into staging. Libraries in `$LFS/lib64/` never reach the artifact. | `find build/sysroot -name 'libpam*'` shows files in `build/sysroot/lib64/`; `find build/staging/sysroot -name 'libpam*'` empty; `build/17-stage-sysroot.sh:72` only `cp -a $LFS/usr`. |
| 2 | dbus exit code 1 | `/etc/machine-id` is absent (skeleton, staging, runtime) AND `/run/dbus/` doesn't exist on the tmpfs-mounted `/run`. dbus-daemon needs both: machine-id at startup, /run/dbus to bind its socket. | `ls build/{skeleton,staging/sysroot}/etc/machine-id` → none; staging `/run/` is empty; tmpfs remount of `/run` would shadow a pre-created `/run/dbus` anyway. |
| 3 | logind D-Bus connect fails (ENOENT) | Symptom of #2 — once `/run/dbus/system_bus_socket` exists, `Connection::system()` in `crates/writeonce-logind/src/main.rs:59` succeeds. No standalone fix needed. | Error message format and code match standard zbus connect-to-missing-socket behaviour. |

### Intended outcome

After this round + an `initramfs` rebuild + reflash, the T450
boots through the bare-minimum chain to a working login prompt on
tty1. No spin lines in dmesg. PAM authentication works.

## Recommended approach

### Fix 1 — libpam (and any other lib*/ stragglers) staged correctly

**Two-part fix; option (a) is essential, (b) is hygiene.**

**(a) Pull `$LFS/lib*/` into staging** — `build/17-stage-sysroot.sh`
step [2/8] currently has only:

```bash
cp -a "$LFS/usr"/. "$STAGING/usr/"
```

Add an explicit copy that merges anything Phase 8 mis-installed
outside `$LFS/usr/` into the UsrMerge target:

```bash
# Some Phase 8 packages (linux-pam observed) configure --libdir=/lib64
# and install libraries to $LFS/lib64/ instead of $LFS/usr/lib/. The
# UsrMerge symlinks (lib64 → usr/lib, lib → usr/lib) don't help
# because nothing is COPIED there. Merge those into usr/lib so the
# symlinks have a real target.
for src in "$LFS/lib64" "$LFS/lib"; do
    if [[ -d "$src" ]]; then
        echo "    merging $src/ → $STAGING/usr/lib/"
        cp -a "$src"/. "$STAGING/usr/lib/" 2>/dev/null || true
    fi
done
```

Place after the existing `cp -a "$LFS/usr"/. "$STAGING/usr/"` so
`$LFS/lib*` overrides nothing already-staged (just adds missing files).

**(b) Long-term: fix the linux-pam build** — `build/08-base-substrate.sh`'s
linux-pam step should pass `--libdir=/usr/lib --bindir=/usr/bin
--sbindir=/usr/sbin` so the package installs to the right places.
Out of scope for this round; tracked for future. Lands the same
files in the same end-state regardless.

### Fix 2 — `/etc/machine-id` populated at install time

`/etc/machine-id` is a one-line UUID file. dbus-daemon requires it
present (the file may be empty, but it must exist). Best practice:
generate per-install, *not* per-image — shipping a fixed UUID in
the skeleton would mean every WriteOnce installation has the same
ID (a real fingerprint/security concern).

Generate it in `crates/writeonce-installer/src/customize.rs`. Add
a new step alongside the existing `rewrite_*` / `apply_network`
functions:

```rust
// In apply():
write_machine_id(mount_root)?;

// New function:
fn write_machine_id(root: &Path) -> Result<()> {
    let id = generate_machine_id_hex();   // 32 hex chars + '\n'
    let etc_path  = root.join("etc/machine-id");
    let dbus_path = root.join("var/lib/dbus/machine-id");
    std::fs::write(&etc_path, &id)?;
    // dbus-daemon falls back to /var/lib/dbus/machine-id if /etc is
    // unreadable, and tools (libdbus, GIO) look at both. Mirror them.
    std::fs::create_dir_all(dbus_path.parent().unwrap())?;
    std::fs::write(&dbus_path, &id)?;
    log::info!("/etc/machine-id and /var/lib/dbus/machine-id: {}",
               id.trim());
    Ok(())
}

fn generate_machine_id_hex() -> String {
    let mut buf = [0u8; 16];
    use std::io::Read;
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf))
        .expect("read /dev/urandom");
    let mut out = String::with_capacity(33);
    for b in &buf {
        out.push_str(&format!("{b:02x}"));
    }
    out.push('\n');
    out
}
```

systemd uses the same format (no dashes, 32 lowercase hex chars).

### Fix 3 — `/run/dbus/` pre-creation in writeonce-pid1

`/run` is mounted as tmpfs by `writeonce-pid1` (via
`crates/writeonce-pid1/src/mount.rs`). Any directory created
*before* the tmpfs mount is shadowed by the empty tmpfs. Right
fix: have writeonce-pid1 create the well-known runtime
subdirectories *after* mounting tmpfs on /run.

Touch `crates/writeonce-pid1/src/mount.rs` — after the loop that
mounts essentials succeeds, add:

```rust
// Create well-known /run subdirectories that services bind sockets
// in. Each gets the owner the corresponding daemon expects (dbus's
// system bus listens at /run/dbus/system_bus_socket as messagebus;
// other services to be added when their failure mode surfaces).
//
// Pre-creation in writeonce-pid1 (not via tmpfiles.d) is the
// minimum-machinery approach. A future writeonce-tmpfiles oneshot
// service can take over once the pattern repeats more than 3 times.
ensure_run_subdir("/run/dbus", 99, 99, 0o755)?;
```

Where `ensure_run_subdir` is a small new helper:

```rust
fn ensure_run_subdir(path: &str, uid: u32, gid: u32, mode: u32) -> io::Result<()> {
    std::fs::create_dir_all(path)?;
    let cstr = std::ffi::CString::new(path).unwrap();
    // SAFETY: chown/chmod with a NUL-terminated path; uid/gid are u32; mode is u32.
    unsafe {
        if libc::chown(cstr.as_ptr(), uid, gid) != 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::chmod(cstr.as_ptr(), mode as libc::mode_t) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
```

Hardcoded UID/GID 99 matches the `messagebus` row in
`build/skeleton/etc/passwd`. Acceptable because (i) those IDs are
also baked into the file already, (ii) the runtime doesn't have a
`getpwnam` shim in PID 1, (iii) systemd does the same hardcoding
in its sd-bus runtime.

## Critical files

| File | Change |
|------|--------|
| `build/17-stage-sysroot.sh` | After `cp -a $LFS/usr/. $STAGING/usr/`, also merge `$LFS/lib64/` and `$LFS/lib/` into `$STAGING/usr/lib/`. |
| `crates/writeonce-installer/src/customize.rs` | New `write_machine_id` function; called from `apply()`. |
| `crates/writeonce-pid1/src/mount.rs` | New `ensure_run_subdir` helper; call it for `/run/dbus` (uid/gid 99, mode 0755) after the `/run` tmpfs mount. |
| `crates/writeonce-pid1/src/mount.rs` (tests) | Verify `ensure_run_subdir` creates the dir with correct mode (running as root in CI, owns the path; not a meaningful chown test without root, so skip the chown assertion). |

No kernel rebuild required. No initramfs Rust crate changes —
fixes are in writeonce-pid1 (PID 1, not the initramfs init binary),
the installer, and the staging script.

## Existing utilities reused

- `crates/writeonce-installer/src/customize.rs::apply` — the pattern
  for "post-extract sysroot mutation" already exists; this is one
  more step in the same shape as `rewrite_passwd`, `rewrite_shadow`,
  etc.
- `crates/writeonce-pid1/src/mount.rs::essentials` — already mounts
  the `/run` tmpfs; we extend that function with the post-mount
  ensure-subdir step.
- `/dev/urandom` for the machine-id UUID — std-only, no new deps.

## Verification

1. **Local unit test for `write_machine_id`.**
   ```
   cargo test -p writeonce-installer --lib customize
   ```
   Add a test that runs `write_machine_id` against a tmpdir and
   asserts the file is 33 bytes (32 hex + newline), all lowercase
   hex, and `/var/lib/dbus/machine-id` mirrors `/etc/machine-id`.

2. **Local unit test for `ensure_run_subdir`** (non-root variant).
   ```
   cargo test -p writeonce-pid1 --lib
   ```
   Run against `/tmp/wo-test-…/run/dbus`; assert the dir exists
   with mode 755. (Skip the chown assertion when not root — but
   verify the call doesn't return Err for an existing dir.)

3. **Workstation libpam staging check.**
   ```
   just stage
   find build/staging/sysroot/usr/lib -name 'libpam*'
   ```
   Expect to see `libpam.so.0`, `libpam.so.0.85.1`, etc.

4. **Workstation machine-id check** (offline; install path runs as
   root over a mounted image). Simulate via a small dry-run:
   ```
   STAGING=$(mktemp -d)
   cp -a build/staging/sysroot/. "$STAGING/"
   # Construct a dummy InstallationPlan and call customize::apply
   # ... or just unit-test write_machine_id in isolation.
   ```

5. **T450 boot path** — `just initramfs` only (no kernel rebuild),
   then `just stage && just artifacts && just usb-install /dev/sda
   && just usb-cmdline-debug /dev/sda`. Expected screen:
   - **No** `libpam.so.0: cannot open` lines.
   - **No** `dbus.service Failed`.
   - **No** `writeonce-logind: InputOutput…NotFound`.
   - `writeonce-login` banner + `login:` prompt on tty1.
   - `login` accepts the user created at install (PAM auth via
     /etc/pam.d/writeonce-login → libpam → /etc/shadow).

## What's NOT in this round

- **No Phase 8a rebuild.** The linux-pam `--libdir` mistake stays
  in the build for now; the staging-side merge covers it
  end-to-end. Fixing the build script properly is a separate,
  lower-priority change.
- **No writeonce-tmpfiles service.** A general-purpose oneshot
  for /run/* dir creation is overkill for one entry. If we add a
  second pre-created /run dir (logind would want
  `/run/systemd/seats/` or equivalent), we'll generalise then.
- **No machine-id regeneration on hardware change.** The
  install-time UUID is stable across reboots; if the user
  ever moves the disk to different hardware, the machine-id stays
  the same. Same as systemd's default — they don't regenerate
  either.
- **No customize tests for dbus integration.** dbus integration
  test would need a full musl-static dbus on the workstation; not
  worth it. The T450 boot is the integration test.

## Cross-references

- [`fix-learn-from-scratch-boot.md`](fix-learn-from-scratch-boot.md)
  — the bare-minimum-boot refit that exposed these as the
  next-blocking issues.
- `.agents/PXL_20260527_213400915.jpg` — boot photo showing the
  three failures + burst-cap firing correctly.
- [`../../docs/learning/t450-boot-debugging.md`](../../docs/learning/t450-boot-debugging.md)
  — running log; this round becomes the next row.
