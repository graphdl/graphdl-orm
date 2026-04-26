/* AREST musl-libc build-feature override for src/internal/version.h
 * (#524).
 *
 * Upstream Makefile generates this header from the top-level VERSION
 * file by shelling out to tools/version.sh. The cc::Build invocation
 * in arest-kernel/build.rs cannot shell out, so we ship a static
 * stamp matching the vendored release (see vendor/musl/VERSION).
 *
 * Refresh hook: bump this string when re-vendoring per
 * vendor/musl/README.md ("How to refresh"). The vendored README's
 * `VERSION` file is the source of truth — keep this matched.
 */
#define VERSION "1.2.5"
