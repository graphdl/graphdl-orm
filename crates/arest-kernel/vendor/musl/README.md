# Vendored musl libc

The complete upstream musl libc source tree lives under
`crates/arest-kernel/vendor/musl/`.

## Upstream

- Project:  musl libc — https://musl.libc.org/
- Version:  **1.2.5** (latest stable at the time of vendoring)
- Source:   https://musl.libc.org/releases/musl-1.2.5.tar.gz
- Maintainer: Rich Felker, et al.

The tarball was downloaded verbatim and extracted into this directory
with the top-level `musl-1.2.5/` wrapper stripped, so the tree is at
`vendor/musl/include/`, `vendor/musl/src/`, `vendor/musl/arch/`, etc.
The full upstream `VERSION`, `WHATSNEW`, `INSTALL`, `Makefile`, and
`configure` files are preserved unchanged.

## License

musl is licensed under the standard **MIT** license — see
`vendor/musl/COPYRIGHT` for the full text and the per-file attribution
of the small number of files under different (compatible) terms.

The MIT license is permissive and compatible with AREST's own
`AGPL-3.0-or-later` umbrella licence (per the FSF compatibility matrix).

## Why vendored?

The vendored tree is the source we build into `libc.a` and `libc.so`
to produce the first canonical Linux ELF binaries that `arest-kernel`
launches inside its sandbox. The build wiring (configure flags,
`cc::Build` invocations, target triple selection) is added separately
in #524; this commit only puts the source on disk.

We pin the version in-tree (rather than fetching at build time) for
the same reasons we vendor Linux 6.6 LTS under `vendor/linux/`:

- reproducible offline builds (Dockerfile.uefi consumes `vendor/`
  unconditionally, no network access from the build container),
- byte-pinned source for security review and provenance,
- ability to apply local patches without forking upstream.

## How to refresh

```powershell
Invoke-WebRequest `
  -Uri https://musl.libc.org/releases/musl-1.2.5.tar.gz `
  -OutFile $env:TEMP\musl.tar.gz
# Then extract into vendor/musl/, stripping the musl-1.2.5/ wrapper.
```

Bump the version above and re-vendor when picking up a newer release.
The `VERSION` file at the root of this directory always reflects the
upstream version that is currently checked in.
