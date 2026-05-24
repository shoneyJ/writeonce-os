//! `writeonce-initramfs` — the Rust /init binary for the WriteOnce initramfs.
//!
//! Boots from the kernel's initramfs, prepares enough userspace to find
//! the real root filesystem, `pivot_root`s into it, and `execve`s
//! `/sbin/writeonce-pid1`. Replaces the BusyBox shell stub from Phase 2.

pub mod cmdline;
pub mod discover;
pub mod modules;
pub mod mount;
pub mod switch_root;
