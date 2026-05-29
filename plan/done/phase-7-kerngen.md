# Phase 7 sub-design — `writeonce-kerngen`: hardware-probe-driven kernel config

> Sub-feature of [`phase-7-kernel.md`](phase-7-kernel.md). Defines a
> Rust tool that derives a minimal kernel `.config` for a specific
> target machine from a hardware probe — replacing the "Ubuntu-config
> + hand-curated fragment" approach we settled for in Phase 8 bring-up.
>
> Status: design only. To be implemented after first successful T450
> boot (likely in Phase 7 work). Don't block on it.

---

## Why

Current state (after T450 bring-up): we ship Ubuntu's `config-6.8.0-117-generic`
as the base, merge our `kernel-config-additions.fragment` on top, and
get a ~15 MB bzImage + ~3000 modules. Most of those modules cover
hardware we'll never have — every Mellanox NIC, every cellular modem,
every Apple-Mac-specific quirk, etc.

That's fine for "one generic kernel for unknown hardware" (Ubuntu's
job). It's wrong for "a kernel built for *this specific T450*" — which
is WriteOnce's positioning.

A precisely-tailored kernel for a known target gives:

- **Faster boot** — no probing of absent hardware
- **Smaller artifacts** — bzImage ~6 MB, modules ~50 MB (vs 400 MB)
- **Faster rebuild** — modules phase drops from 30+ min to ~3 min
- **Smaller attack surface** — code that doesn't exist can't be a CVE
- **Educational legibility** — every CONFIG enabled has a justification
  traceable to a hardware ID

The goal is to make this derivation **mechanical**, not hand-curated.

---

## Source of truth (three layers)

### 1. `MODULE_DEVICE_TABLE` macros in kernel source

Ground truth. Every driver declares the device IDs it claims:

```c
static const struct pci_device_id e1000e_pci_tbl[] = {
    { PCI_VDEVICE(INTEL, 0x1502), board_pch_lpt },
    { PCI_VDEVICE(INTEL, 0x153a), board_pch_lpt },
    ...
};
MODULE_DEVICE_TABLE(pci, e1000e_pci_tbl);
```

### 2. `modules.alias` — depmod's compiled index

After `make modules_install`, `depmod` scans every `.ko`'s
`MODULE_DEVICE_TABLE` and writes:

```
alias pci:v00008086d00001502sv*sd*bc*sc*i* e1000e
alias pci:v00008086d0000153Asv*sd*bc*sc*i* e1000e
alias pci:v00008086d00001616sv*sd*bc*sc*i* i915
```

This is the lookup index. Wildcards (`v*`, `sv*`, etc.) encode partial
matches. Given a target's actual modalias strings from `/sys`, you
have a deterministic mapping to module names.

### 3. linuxhw.io — community probe corpus

[linux-hardware.org](https://linux-hardware.org) collects probes from
~500k machines. ThinkPad T450 has thousands of probes. Tells you not
just what the kernel *claims* to support, but what *actually works* in
the field — modules that load but fail, firmware that's missing, etc.

Used as a sanity check: "every other T450 user is loading these
N modules; ours should too".

---

## Algorithm

```
1. Probe (run on target, or feed pre-collected files):

    for entry in /sys/bus/{pci,usb,acpi,virtio,platform}/devices/*:
        read modalias        # "pci:v00008086d00001616sv00001028sd00000620bc03sc00i00"
        record device id

    also: /proc/cpuinfo for CPU features (sse4_2, aes, rdrand, ...)
    also: /sys/firmware/efi/* for UEFI presence
    also: /sys/class/dmi/id/* for laptop model

2. Resolve modaliases → modules:

    open /lib/modules/<ver>/modules.alias
    for each probed modalias:
        find matching `alias <pattern> <module>` line(s)
            (wildcards: simple shell-glob-style matching, in-tree
             algorithm is in kernel/module/main.c)
        record module name

3. Resolve module → CONFIG_* :

    options:
    (a) Parse Kconfig tree, walk select/depends on graph.
        ~500 source files; existing parsers (`kconfig-hardened-check`'s
        `kconfig` lib in Python) are usable references but not Rust-native.
    (b) Maintain a hand-curated `module → CONFIG_*` map regenerated
        from kernel source via a one-off `grep + script` pass. Cheap.
    (c) Let the kernel's own `kconfig` solver do it: emit a stub
        Kconfig fragment with `CONFIG_FOO=m` for each module, then run
        `make olddefconfig` — kernel's solver pulls in dependencies.

    Recommend (c) — outsources the hard part to the kernel itself,
    same path `make localmodconfig` takes.

4. Add CPU-feature CONFIGs:

    /proc/cpuinfo flags → x86 CONFIG_* (CONFIG_X86_AVX2, CONFIG_X86_AES_NI, ...)
    DMI vendor/model → laptop-quirk CONFIGs (THINKPAD_ACPI, ...)
    UEFI presence → CONFIG_EFI, CONFIG_EFI_STUB, ...

5. Emit fragment:

    # writeonce-kerngen — derived 2026-05-26T...
    # Target: Lenovo ThinkPad T450, i5-5300U, UEFI Aptio V
    # Source: probe collected from /sys on the T450 itself
    CONFIG_DRM_I915=m
    CONFIG_E1000E=m
    CONFIG_IWLMVM=m
    CONFIG_IWLWIFI=m
    ...
```

---

## Comparison with existing tools

| Tool | What it does | Why we don't use it as-is |
|---|---|---|
| `make localmodconfig` | Generates config from `lsmod` on the running system | Misses cold-plugged hardware. Requires already-booting machine — chicken-and-egg for first install. |
| `autokernel` ([oddlama/autokernel](https://github.com/oddlama/autokernel)) | Python; reads `/sys`, walks Kconfig, generates fragment | The architecture is exactly what we want. Reasons to write our own: language consistency with WriteOnce stack (Rust); design space for cross-host probe (probe on T450, generate config on the workstation). |
| NixOS `nixos-generate-config` | Generates `hardware-configuration.nix` from probe | Nix-language-locked; doesn't emit Kconfig fragments. |
| Buildroot / Yocto BSPs | Per-board hand-curated kernel configs | Manual; no probe-driven derivation. |
| Clear Linux platform auto-tuning | Per-CPU-family kernel variants | Closed magic; not reusable. |

---

## Language choice

**Rust.** Reasons:

- Stack-consistent with `writeonce-pid1/svc/login/logind/installer`.
- `/sys` and `/proc` parsing is std-friendly.
- Could ship inside the installer (cross-build kernel config at
  install time based on the *target's* hardware, not the workstation's).
- Type-safe: modaliases, Kconfig symbols, device IDs all distinguishable.
- Single static-musl binary — fits the existing `target/x86_64-unknown-linux-musl/release/`
  layout.

**Why not Python**: existing `autokernel` shows this works in Python.
But adding Python to WriteOnce's runtime is a Phase 8 dep we don't
currently have. Rust is already on the path.

---

## Phased implementation

| Phase | Deliverable | Effort |
|---|---|---|
| 7a | `writeonce-kerngen probe` — collect hardware data into JSON file. Run on target, save to `~/t450-probe.json`. | 1 day |
| 7b | `writeonce-kerngen resolve` — given a probe JSON + kernel source tree, emit a Kconfig fragment. Uses approach (c) above: stub fragment → `make olddefconfig` → diff. | 2-3 days |
| 7c | Integration: replace `kernel-config-additions.fragment` with a derived-fragment workflow. `build/04-kernel.sh` learns to call `writeonce-kerngen` if a probe is available. | 1 day |
| 7d | Verification: build per-target kernel, boot it on T450, compare bzImage size + boot time + lsmod against the Ubuntu-config baseline. Document numbers in `docs/learning/`. | 1 day |
| 7e | Multi-target support: `writeonce-kerngen` accepts probe files for multiple targets; emits N derived kernels OR a single "union" kernel with all needed CONFIGs. | 2-3 days |

Total ~2 weeks calendar to a useful first version. Worth doing once
the project has a known-good baseline to compare against.

---

## Open questions

- **Firmware blobs.** `iwlwifi` needs `iwlwifi-7265D-29.ucode` in
  `/lib/firmware`. Our tool can identify *which* firmware files are
  needed (from the kernel's `MODULE_FIRMWARE` macros) but the blobs
  themselves come from `linux-firmware.git`. Cross-reference at probe
  time.
- **Kernel command-line tuning.** Some hardware quirks need a kernel
  cmdline arg, not a CONFIG (e.g. `i915.fastboot=0`). Probe data
  alone doesn't tell us these — they live in distro maintenance
  knowledge. Bridge via the linuxhw.io community probe data.
- **Forward compatibility.** A kernel upgrade can rename / split /
  merge CONFIG symbols. `writeonce-kerngen` produces a fragment
  pinned to a specific kernel version; bumping the kernel may
  invalidate the fragment.
- **Module vs built-in (=y vs =m).** `make localmodconfig` defaults
  everything to `=m`. For an embedded-style WriteOnce build, we may
  want most things `=y` to avoid the initramfs needing modules at
  all. Configurable per-output.

---

## Cross-references

- [`plan/done/phase-7-kernel.md`](phase-7-kernel.md) — overall Phase 7 design
- [`docs/learning/t450-boot-debugging.md`](../../docs/learning/t450-boot-debugging.md) — the bring-up pain that motivated this design
- `kernel-base.config` + `kernel-config-additions.fragment` — current
  hand-curated approach this will eventually supersede
- linuxhw.io — community probe corpus
- [autokernel](https://github.com/oddlama/autokernel) — reference impl in Python
