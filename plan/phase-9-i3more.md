# Phase 9 — i3More integration + login/DM flow

**Goal.** A user logs in on the T450; i3More runs as the desktop experience.

## Subtasks

1. **Build i3More from `../.agents/reference/i3More/`** using the cross-toolchain + GTK4 + PAM + PipeWire from Phase 8. Match i3More's Cargo.toml feature flags; start with the `lock`, `audio`, `launcher` features. Defer `speech-text` (CUDA dep doesn't exist on this T450).

2. **Install i3More binaries** into `/usr/local/bin/`. Write their service files / autostart entries for i3.

3. **Login flow — design decision.** Two options:
   - **(a)** Console getty on tty1 → user types `wo-login` → PAM auth → starts `dbus-session`, `pipewire`, `wireplumber`, `Xorg`, `i3`, `i3more`. Simplest; matches the learn-by-building ethos.
   - **(b)** Graphical DM (small Rust+GTK4 binary). More work but the i3More README mentions a GTK4 DM aspiration.
   - **Recommendation:** ship (a) for first user-facing milestone; build (b) as Phase 9b once (a) is stable.

4. **Write `wo-login`** as a small Rust binary: PAM `pam_authenticate` + `pam_acct_mgmt` + `pam_setcred` + `pam_open_session`, then `setuid`/`setgid`, `execve` into a session-starter script (or directly into a session-manager binary).

5. **Verify i3More's logind dependencies.** `i3more-lock` calls `org.freedesktop.login1.Manager.Inhibit`. Test against the Phase-4 shim; expand the shim's surface if needed (re-run the OS-dep survey).

6. **Per-user state.** `/home/<user>/.config/i3/config`, `/home/<user>/.config/i3more/`. Seed via `/etc/skel/`.

7. **Test on T450.** Reboot → `wo-login` prompt → enter creds → i3 starts → `Mod+Enter` opens a terminal → `i3more` floating bar visible → audio keybinds work via `i3more-audio` → screen lock with the configured hotkey.

8. **(Phase 9b — graphical DM).** Defer until 9 is solid. Rust + GTK4, runs as a service in `graphical.target`, reuses `wo-login`'s PAM logic. Mentioned in `../writeonce-session-notes.md` cgroup diagram as `display-manager.service`.

## Deliverable

Power on T450 → boot → login prompt → enter creds → i3 desktop with i3More running.

## Acceptance criteria

- A cold reboot lands at a usable desktop in < 30 seconds.
- Locking the screen via `i3more-lock` actually locks; unlocking via PAM works.
- `i3more-audio` adjusts volume via PipeWire.
- The boot order matches the cgroup diagram in `../writeonce-session-notes.md` Topic 2 (adjusted: no Sway/smithay; X11 + i3 instead).

## References

- `../.agents/reference/i3More/` end-to-end.
- The i3More OS-dependency survey (`docs/learning/i3more-os-deps.md` — to be authored at the start of this phase from a fresh re-survey).
