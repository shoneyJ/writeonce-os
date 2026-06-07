# Could WriteOnce have its own package manager? (feasibility study)

A learning exploration, **not a plan to build one**. The project scope
(`project-writeonce-scope`) is explicit: *package management is **Nix**,
single-user, adopted wholesale — no bespoke package manager, no authored package
definitions.* The bespoke surface is the **boot path** (the Rust crates + the
kernel config), nothing more. This doc asks "what *would* a mini WriteOnce PM
take?" purely to understand the machinery we get for free from Nix — and to record
why reinventing it is out of scope.

## What any real package manager must solve

| Concern | Naïve PM (apt/dpkg-style) | Nix |
|---|---|---|
| **Where files go** | shared `/usr` — files from different pkgs collide; one version at a time | `/nix/store/<hash>-<name>/` — immutable, hashed by inputs; many versions coexist, no collisions |
| **Identity** | name + version string | cryptographic hash of *all* build inputs |
| **Dependencies** | declared ranges, resolved at install (SAT solver, conflicts) | exact closure baked into each store path; no resolution, no conflicts |
| **Upgrade/rollback** | mutate `/usr` in place; rollback is hard | new store paths + a new *profile generation*; rollback = flip a symlink |
| **Atomicity** | partial installs possible mid-failure | one `rename(2)` of the profile symlink — all-or-nothing |
| **Removal/GC** | refcounts, dangling files | mark-and-sweep GC from profile *roots* |
| **Binary distribution** | repo of `.deb`s | substituters serve signed NARs of store paths |

The hard, interesting problems — collisions, atomic upgrade, rollback, GC — are
solved *by the content-addressed store + profiles*, which is exactly Nix's design
(see `nix-profile-internals.md`).

## A toy `wo-pkg` — the minimum that captures the idea

If one wanted the *shape* of Nix in ~150 lines (illustrative only):

```
/wo/store/<sha256>-<name>/        # immutable, hash-named (here: hash of the source)
/wo/profiles/profile-<N>/         # symlink forest: bin/, share/ → store paths
/wo/profiles/current -> profile-N # atomic switch = re-point this symlink
/wo/gcroots/                      # what keeps store paths alive
manifest.toml                     # per profile: name → store path
```

- `wo-pkg add <url> <sha256>`: fetch tarball → verify sha → unpack to
  `/wo/store/<sha256>-<name>` (skip if present) → rebuild a new `profile-<N+1>`
  symlink forest including it → flip `current` → record in `manifest.toml`.
- `wo-pkg rollback`: re-point `current` to `profile-<N-1>`.
- `wo-pkg gc`: delete `/wo/store/*` not reachable from any profile under
  `/wo/gcroots`.

That's a content-addressed store + generations + GC — a *toy Nix*. ~50–150 lines
of Rust or POSIX shell.

## What the toy still lacks (and why it's not worth building)

The toy hashes the *source*, not the build inputs, and has **no dependency
closure, no build sandbox, no language to express packages, no binary cache, no
signing, no multi-output, no reproducibility guarantees**. Adding those *is*
re-implementing Nix — years of work, and squarely against the scope. WriteOnce
already ships Nix (Phase 14 / W3), which provides all of the above, battle-tested.

## Verdict

- **Keep Nix** as the package manager. It is the from-login userspace tier; the
  bespoke surface stays the boot path.
- The *learning* value here — understanding content-addressed stores, profiles,
  generations, atomic rollback, and GC roots — is fully delivered by this doc plus
  [`nix-profile-internals.md`](nix-profile-internals.md). No code to write.
- If a `wo-pkg` toy is ever built, it should live as an explicitly-labelled
  **learning experiment** (e.g. `experiments/wo-pkg/`), never as the OS's real
  package path, and only after the user re-opens the scope decision.
