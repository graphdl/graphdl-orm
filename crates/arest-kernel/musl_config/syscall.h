/* AREST musl-libc syscall-number override (#524).
 *
 * Pulled in via `-include musl_config/syscall.h` in build.rs's
 * cc::Build invocation AFTER musl's own arch/x86_64/bits/syscall.h.in
 * (so `__NR_xxx` macros are already defined). Empty today because
 * AREST's tier-1 x86_64 syscall numbering is identical to Linux's:
 * the `syscall` instruction in musl's __syscallN inline-asm thunks
 * (see vendor/musl/arch/x86_64/syscall_arch.h) jumps into AREST's
 * SYSCALL/SYSRET handler, which routes by RAX through the same
 * Linux x86_64 ABI table the vendored bits/syscall.h.in defines
 * (entries 0..334 for the classic range, 424..452 for the io_uring
 * / pidfd / openat2 cluster).
 *
 * When AREST diverges from Linux (a syscall outside Linux's
 * allocation, or a remapping of an unused number for AREST-specific
 * functionality), the override lands here as
 *   #undef __NR_xxx
 *   #define __NR_xxx <AREST number>
 * pairs. Because this header is included AFTER the upstream
 * syscall.h.in via cc::Build's -include flag list, the redefine wins.
 *
 * Reference: docs/16-uefi-pivot.md (kernel surface) and the eventual
 * #507 / #497-#502 syscall-implementation tracks (none of which have
 * landed yet — until then, missing syscalls in the link step will
 * unresolve at runtime, not at musl-build time).
 */
#ifndef AREST_MUSL_CONFIG_SYSCALL_H
#define AREST_MUSL_CONFIG_SYSCALL_H

/* Reserved for future syscall-number divergence from Linux. */

#endif /* AREST_MUSL_CONFIG_SYSCALL_H */
