# Login greeter + session chooser (`writeonce-greeter`)

WriteOnce boots `systemd-wayland-hyprland` to a **password-gated TUI greeter**
on tty1 (`writeonce-greeter`). You sign in, pick a session from a menu (the
"gear" / ⚙ list) — an installed Wayland compositor or a plain **Shell (bash)** —
and it launches the chosen session as you. tty2–tty6 are normal password gettys
(a shell fallback); SSH stays key-based. There is no autologin: **bash and the
desktop both require a password**.

This replaces the previous autologin-to-console boot. It's a from-scratch Rust
crate, not greetd/GDM — a text greeter needs nothing but the VT.

## Why a TUI (and why it's simpler than greetd)

A *graphical* greeter is itself a Wayland client and needs a compositor to render
on (greetd + ReGreet-in-cage), plus logind seat access *for the greeter*. A
**text** greeter just `write()`s to `/dev/tty1`, so it needs **no DRM and no
logind session of its own**. That collapses the design to a single PAM
session — opened *for the user* — exactly like `writeonce-login`, with a session
selector and the env hints bolted on. The compositor takes DRM master via logind
once it starts; the greeter never touches KMS/VT ioctls.

## The login flow

```
writeonce-greeter.service (tty1, Conflicts=getty@tty1, Restart=always)
  loop:
    render banner; read username (default writeonce)
    look up the user; build the menu = discovered Wayland sessions + "Shell (bash)"
    pick a session (numbered; remembers the last choice)
    pam_start("writeonce-greeter")
      set_item(PAM_TTY, "tty1")                  # bare name, NOT /dev/tty1
      putenv XDG_SEAT=seat0, XDG_VTNR=1,
             XDG_SESSION_CLASS=user,
             XDG_SESSION_TYPE=wayland|tty,        # BEFORE open_session
             XDG_SESSION_DESKTOP=<DesktopNames[0]>
      authenticate → acct_mgmt → setcred(ESTABLISH) → open_session
                                                   #   pam_systemd → logind:
                                                   #   creates the session on
                                                   #   seat0/VT1 + /run/user/<uid>
    env = pam_getenvlist()                        # XDG_RUNTIME_DIR, XDG_SESSION_ID, DBUS_…
    fork:
      child:  drop_privileges(uid,gid,user) → chdir($HOME)
              env = base_env ∪ pam_env            # our XDG_* win; pam adds RUNTIME_DIR
              compositor:  exec  bash -lc 'exec <Exec>'   # login shell → Nix profile on PATH
              shell:       exec  -bash                    # login shell
      parent: waitpid → close_session → delete_cred → loop
```

**Key ordering facts:**
- The `XDG_*` hints are set with `pam_putenv` **before** `pam_open_session`, so
  `pam_systemd` classifies the logind session correctly (graphical wayland on
  seat0/VT1). Set after, they'd be ignored.
- `PAM_TTY` is the bare name `"tty1"`, never `/dev/tty1`.
- `pam_getenvlist()` **after** `open_session` is how `XDG_RUNTIME_DIR` /
  `XDG_SESSION_ID` / `DBUS_SESSION_BUS_ADDRESS` reach the session env. The child
  merges them, our explicit keys winning on conflict.
- The compositor is launched via the user's **login shell** (`bash -lc 'exec …'`)
  so `/etc/profile.d/nix.sh` puts `~/.nix-profile/bin` on PATH and a bare `Exec`
  (e.g. `sway`) resolves.

## Session discovery (the menu)

Sessions are freedesktop `*.desktop` files (the same ones GDM/SDDM/greetd read),
scanned from — in priority order — `/usr/share/wayland-sessions`,
`~/.nix-profile/share/wayland-sessions`,
`/nix/var/nix/profiles/default/share/wayland-sessions`, and
`~/.local/share/wayland-sessions`. A compositor's nixpkgs package ships its own
`share/wayland-sessions/<name>.desktop`, so **installing it via Nix makes it
appear** — "discovered" == "installed", no extra probe. `Name=`/`Exec=`/`TryExec=`/
`DesktopNames=` are parsed; `Exec` field codes (`%U`, `%f`, …) are stripped. A
synthetic **Shell (bash)** entry is always appended (and is the lockout backstop).
The last choice is remembered in the root-owned `/var/lib/writeonce-greeter/last`
(never written into `$HOME` from the root greeter).

## Installing a desktop: the `.nix` templates

WriteOnce ships per-compositor template flakes under `/etc/writeonce/sessions/`:
`sway/flake.nix` and `hyprland/flake.nix` (selection manifests — they pick the
compositor + a starter set out of nixpkgs). Workflow:

```sh
# Fresh boot → the greeter offers only "Shell (bash)". Log in to it, then:
nix profile add path:/etc/writeonce/sessions/sway      # or: wo-install-session sway
#   (or any nixpkgs compositor directly, incl. Plasma/KWin:)
nix profile add github:NixOS/nixpkgs/nixos-unstable#sway
# log out → "Sway" now appears at the greeter → pick it.
```

`nix flake lock` inside a template dir pins the nixpkgs rev for reproducibility.
Any compositor works (sway / hyprland / wayfire / Plasma) — the templates are
just convenience for the two requested.

## The `pam_service` ↔ PAM-file contract (read before changing)

`writeonce-greeter` calls `pam_start("writeonce-greeter")`, so
`/etc/pam.d/writeonce-greeter` **must exist** — if the service name and the file
disagree, PAM falls back to `other` (deny-by-default) and **every local login
fails**. The greeter's default `pam_service` and `greeter.toml` both say
`writeonce-greeter`, matching the shipped file. The PAM stack mirrors
`/etc/pam.d/login`: `pam_unix` (auth/account/session) + `session optional
pam_systemd.so` (`optional` so a logind hiccup degrades to a no-runtime-dir
session rather than blocking auth — you can still pick Shell and debug).

## Lockout-safety invariants (don't regress these)

1. `writeonce-greeter`'s default `pam_service` == the shipped
   `/etc/pam.d/writeonce-greeter` file name.
2. The service is `Restart=always`; the loop never exits on a recoverable error;
   the synthetic **Shell** entry is always present even with zero compositors.
3. On `exec` failure the child `_exit(127)` and the greeter re-renders; it never
   leaves the VT in graphics mode (the greeter sets no KMS mode).
4. `install.sh` **hard-fails** if it can't set a password — with autologin gone,
   a locked account means no local login.
5. tty2–tty6 password gettys (logind autovt) and key-based SSH remain as escape
   hatches; don't disable autovt.
6. `17-stage-sysroot.sh` asserts (`[4a/8]`) that whenever the greeter unit is
   staged, the binary + PAM file + `multi-user.target.wants` symlink are present
   and `getty@tty1` is absent.

## Where it lives

| Piece | Path |
| ----- | ---- |
| greeter crate | `crates/writeonce-greeter/` (`config.rs`, `sessions.rs`, `main.rs`) |
| shared PAM env FFI + privdrop | `crates/writeonce-login/src/{pam.rs,session.rs}` (reused) |
| service + enablement | `…/etc/systemd/system/writeonce-greeter.service` (+ `multi-user.target.wants/`) |
| PAM stack | `…/etc/pam.d/writeonce-greeter` |
| greeter config | `…/etc/writeonce/greeter.toml` |
| `.nix` templates + helper | `…/etc/writeonce/sessions/{sway,hyprland}/flake.nix`, `…/usr/local/bin/wo-install-session` |
| staging + lockout guard | `build/17-stage-sysroot.sh` (`[3a/8]`, `[4a/8]`) |
| install-time password | `build/install.sh` (`[5/6]`, now hard-fails) |

(`…` = `build/skeleton/systemd-wayland-hyprland/`.) Removed: the old autologin
(`getty@tty1.service.d/autologin.conf`) and the static `getty.target.wants/
getty@tty1.service`.

## Verification

- **Host (here):** `cargo build -p writeonce-greeter --release` + `cargo test`/
  `clippy` (the `.desktop` parser, field-code stripping, config defaults) — all
  green. PAM/VT/launch + Nix `.nix` eval are target/Nix-gated.
- **Target (after reflash, keep an SSH session open as a backstop):** tty1 shows
  the greeter; password required; **Shell (bash)** works before any compositor is
  installed; `nix profile add path:/etc/writeonce/sessions/sway` → "Sway" appears
  → selecting it launches with DRM; `loginctl` shows seat0/wayland/active +
  `/run/user/1000`; on compositor exit the greeter re-renders; Ctrl+Alt+F2 → a
  password getty → bash.
