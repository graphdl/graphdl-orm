# AREST-specific musl config overrides (#524)

This directory holds the small set of headers that the `musl-libc`
build feature pre-pends to musl's own header search path so the
vendored musl 1.2.5 source tree (`crates/arest-kernel/vendor/musl/`)
can be compiled against AREST's syscall surface instead of an
external Linux kernel.

## Files

- `version.h` — drops in for `obj/src/internal/version.h`, which the
  upstream Makefile generates by shelling out to `tools/version.sh`.
  We can't shell out from `cc::Build`, so we ship a static stamp
  matching the vendored release.

- `arch.h` — opt-in arch override pulled in via `-include`. Empty
  today; reserved for future per-arch tweaks (e.g. clamping
  `SYSCALL_RLIM_INFINITY` to AREST's chosen rlimit ceiling).

- `syscall-override.h` — opt-in syscall override pulled in via
  `-include`. Empty today because AREST's tier-1 x86_64 syscall
  numbering is identical to Linux's (per the `vendor/musl/arch/x86_64/
  bits/syscall.h.in` table). When AREST diverges (e.g. adds a syscall
  outside Linux's allocation, or remaps an unused number), the
  override goes here as `#undef __NR_xxx` + `#define __NR_xxx ...`
  pairs, picked up after musl's per-arch syscall.h.

  NOTE the name: it is deliberately NOT `syscall.h`. `-Imusl_config`
  is first on the include search path (so `version.h` below
  substitutes for the build-generated `src/internal/version.h`), which
  means a file named `syscall.h` here would be found by the ~296 musl
  sources that `#include "syscall.h"` BEFORE musl's real
  `src/internal/syscall.h` — shadowing it and stripping `__syscall` +
  every `SYS_*` from those translation units. Force-include works by
  any name; the non-colliding name keeps it additive instead of a
  shadow.

## Why a separate directory rather than patching `vendor/musl/`?

The `vendor/musl/` tree is a verbatim drop of the upstream tarball
(per WWWW's #523 vendoring note in `vendor/musl/README.md`). Keeping
AREST's overrides outside that tree means:

1. Refreshing the upstream version (re-running the `Invoke-WebRequest
   ... -OutFile $env:TEMP\musl.tar.gz` recipe in the README) is a
   plain `Remove-Item -Recurse vendor/musl; Expand-Archive ...` —
   no patch-stack to rebase.
2. Provenance: the SHA of every file under `vendor/musl/` matches the
   upstream tarball, so an SBOM tool grovelling the build artifact
   tree can attest to the vendored libc's origin without flagging
   any file as "modified upstream".
3. The build script's include-path order (`-Imusl_config -Ivendor/
   musl/arch/x86_64 -Ivendor/musl/include -Ivendor/musl/src/...`) lets
   `version.h` here substitute for the build-generated
   `src/internal/version.h` (which AREST does not generate). Because
   `-Imusl_config` is FIRST, override files MUST NOT share a name with
   a real musl header on the search path, or they shadow it (see the
   `syscall-override.h` note above). Genuine overrides are applied via
   the `-include` force-include + `#undef`/`#define`, not by
   search-path shadowing.
