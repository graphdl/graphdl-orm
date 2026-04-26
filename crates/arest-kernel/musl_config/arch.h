/* AREST musl-libc per-arch override (#524).
 *
 * Pulled in via `-include musl_config/arch.h` in build.rs's
 * cc::Build invocation BEFORE musl's own arch headers. Empty today —
 * AREST's tier-1 x86_64 arch matches what musl already expects (SysV
 * AMD64 ABI, 64-bit `long`, syscall instruction). Future overrides
 * (e.g. clamping SYSCALL_RLIM_INFINITY to a kernel-policy ceiling)
 * land here as `#define`-before-include guards.
 *
 * See musl_config/README.md for the rationale on keeping AREST
 * overrides out of the verbatim vendor/musl/ tree.
 */
#ifndef AREST_MUSL_CONFIG_ARCH_H
#define AREST_MUSL_CONFIG_ARCH_H

/* Reserved for future per-arch overrides. */

#endif /* AREST_MUSL_CONFIG_ARCH_H */
