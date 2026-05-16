# Phase 7 — Kernel customization & Rust kernel module experiment

**Goal.** Move from "we use mainline as-is" to "we own our kernel config and can build modules in Rust." Optional ambitious end: a small upstream patch.

## Subtasks

1. **Freeze kernel 6.12.x.** Track it as a vendored fork in `vendor/linux/` or as a remote branch reference; record exact SHA in `versions.env`.

2. **Audit the kernel config end-to-end.** Walk every `=y` and `=m`; document *why* each is enabled tied to `../.agents/target-machine.md`. Anything not justified → disable. This is the learning beat the user wants.

3. **Enable `CONFIG_RUST=y`.** Kernel 6.12 has Rust support since 6.1; toolchain requirements documented in `../.agents/reference/linux/Documentation/rust/quick-start.rst`. Match rustc + libclang versions exactly to what the kernel demands.

4. **Build the `rust_minimal` sample module** that ships in `samples/rust/`. Verifies the kernel-Rust toolchain.

5. **Author a small WriteOnce Rust module.** Candidate: a `/dev/writeonce-thermal` char device that reads from the Wildcat Point-LP Thermal Management Controller (`8086:9ca4`, driver `intel_pch_thermal`) and exposes a JSON line per read. Pure learning exercise — i3More doesn't need it.

6. **Build the kernel + module against the cross-toolchain.** Install module to `/lib/modules/6.12.x/extra/writeonce-thermal.ko`. Modprobe from the supervisor.

7. **Configure stable runtime knobs.** `sysctl.conf` equivalent — Rust supervisor reads `/etc/writeonce/sysctl.toml` at boot and writes to `/proc/sys`.

8. **(Stretch)** Find one trivially fixable warning, typo, or doc nit in the kernel tree and prepare a patch. Even a `checkpatch.pl` clean-up of comments in a Broadwell-adjacent driver is a real contribution. Submit via `git format-patch` + `git send-email` to the appropriate maintainer (`scripts/get_maintainer.pl`).

## Deliverable

A WriteOnce-specific kernel config (`build/kernel-config`), a small Rust kernel module loaded at boot, and a documented config rationale.

## Acceptance criteria

- `cat /sys/kernel/rust_*` or equivalent shows Rust support active.
- `lsmod | grep writeonce_thermal` after boot.
- `cat /dev/writeonce-thermal` returns valid JSON with the current CPU package temp.
- `../.agents/kernel-config-rationale.md` explains every non-default config switch.

## References

- `../.agents/reference/linux/Documentation/rust/` — Rust kernel docs.
- `../.agents/reference/linux/samples/rust/` — example modules to crib from.
