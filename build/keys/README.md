# GPG keys for upstream source verification

`fetch.sh` verifies the detached GPG signature shipped alongside each tarball
where one is available. This directory holds the public keys it trusts.

Populate it on first run by importing the relevant key for each upstream:

```bash
# Linux kernel — signed by Linus Torvalds and Greg KH (the kernel release manager)
gpg --keyserver keyserver.ubuntu.com --recv-keys 79BE3E4300411886 38DBBDC86092693E
# Binutils, GCC — signed by the FSF / GNU release managers
gpg --keyserver keyserver.ubuntu.com --recv-keys 13FCEF89DD9E3C4F   # GCC 14.x
gpg --keyserver keyserver.ubuntu.com --recv-keys 3A24BC1E8FB409FA9F14371813FCEF89DD9E3C4F  # binutils
# glibc
gpg --keyserver keyserver.ubuntu.com --recv-keys 1A85FFC6D88E42B4
# BusyBox
gpg --keyserver keyserver.ubuntu.com --recv-keys C9E9416F76E610DBD09D040F47B70C55ACC9965B
```

Then export the keys to this directory so the build is reproducible without
your personal GPG keyring:

```bash
gpg --export --armor 79BE3E4300411886 > linux-torvalds.asc
gpg --export --armor 38DBBDC86092693E > linux-gregkh.asc
# ...and so on for each key
```

`fetch.sh` will `gpg --import` everything in this directory into a project-local
keyring (`build/gnupg/`) on each run; no global keyring pollution.

**Note:** keys do rotate. If verification fails after a version bump, check the
upstream announcement for a key change before assuming the tarball is tampered.
