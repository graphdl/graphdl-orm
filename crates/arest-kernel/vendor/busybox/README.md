# Vendored busybox

The complete upstream busybox source tree lives under
`crates/arest-kernel/vendor/busybox/`.

## Upstream

- Project:  busybox — https://busybox.net/
- Version:  **1.36.1** (latest 1.36.x stable at the time of vendoring)
- Source:   https://busybox.net/downloads/busybox-1.36.1.tar.bz2
- Maintainer: Denys Vlasenko, et al.

The tarball was downloaded verbatim and extracted into this directory
with the top-level `busybox-1.36.1/` wrapper stripped, so the tree is
at `vendor/busybox/applets/`, `vendor/busybox/coreutils/`,
`vendor/busybox/libbb/`, etc. The full upstream `LICENSE`, `README`,
`AUTHORS`, `Makefile`, `INSTALL`, `Config.in`, and `TODO` files are
preserved unchanged.

## License

busybox is licensed under the **GPL-2.0-only** license — see
`vendor/busybox/LICENSE` for the full text. Note this is GPL **v2
only** (no "or later" clause), per upstream's explicit statement at
the top of `LICENSE`:

> BusyBox is distributed under version 2 of the General Public
> License [...]. Version 2 is the only version of this license which
> this version of BusyBox (or modified versions derived from this
> one) may be distributed under.

## License compatibility note

GPL-2.0-only is **not** straightforwardly compatible with AREST's
default `AGPL-3.0-or-later` umbrella licence (per the FSF
compatibility matrix: AGPL-3.0-or-later code can ONLY incorporate
GPL-2.0-or-later or GPL-3.0-or-later code, not GPL-2.0-only).

Per the #396 license decision (recorded in
`crates/arest-kernel/doom_assets/README.md`), the user has accepted
GPL feature-gating: GPL-only artefacts ship behind a Cargo feature
flag (`busybox` here, like `doom` for `jacobenget/doom.wasm`) so the
default kernel build remains pure AGPL-3.0-or-later. Opting in via
`--features busybox` brings in the busybox build pass and embeds the
resulting static binary into a separate artefact (`$OUT_DIR/busybox`,
NOT linked into the kernel `.efi` image at this commit).

The `.efi` kernel image stays AGPL-3.0-or-later. The busybox binary
is GPL-2.0-only and shipped alongside it as a guest payload — same
licensing pattern Linux distros use for GPL userland on top of a
permissive kernel surface.

## Why vendored?

The vendored tree is the source we build into a static x86_64-linux
ELF binary that `arest-kernel` launches inside its sandbox via the
ELF loader (#472, when fully wired). The build wiring (configure
flags, `cc::Build` invocations, target triple selection) lives in
`crates/arest-kernel/build.rs`'s `build_busybox` pass; this commit
puts the source on disk and lights up the build path.

We pin the version in-tree (rather than fetching at build time) for
the same reasons we vendor musl 1.2.5 under `vendor/musl/` and Linux
6.6 LTS under `vendor/linux/`:

- reproducible offline builds (Dockerfile.uefi consumes `vendor/`
  unconditionally, no network access from the build container),
- byte-pinned source for security review and provenance,
- ability to apply local patches without forking upstream.

## Configuration

The build pass selects a minimal applet set — only `ls`, `cat`,
`echo`, `wc`, `head`, and `tail` — by reading
`crates/arest-kernel/busybox_config/.config`. That file disables every
other applet via `# CONFIG_<APPLET> is not set` lines and enables
the six chosen ones via `CONFIG_<APPLET>=y`.

Keeping the config OUT of `vendor/busybox/` (in a sibling
`busybox_config/` directory mirroring the `musl_config/` pattern that
DDDDD established for the musl build in #524) means refreshing the
upstream version is a plain `Remove-Item -Recurse vendor/busybox;
Expand-Archive ...` — no patch-stack to rebase.

## How to refresh

```powershell
Invoke-WebRequest `
  -Uri https://busybox.net/downloads/busybox-1.36.1.tar.bz2 `
  -OutFile $env:TEMP\busybox.tar.bz2
# Then extract into vendor/busybox/, stripping the busybox-1.36.1/ wrapper.
```

Bump the version above and re-vendor when picking up a newer release.
The vendored `Makefile`'s `VERSION`/`PATCHLEVEL`/`SUBLEVEL` lines at
the top always reflect the upstream version that is currently checked
in.
