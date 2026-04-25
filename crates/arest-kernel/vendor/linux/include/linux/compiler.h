/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/compiler.h` for the AREST linuxkpi shim. Real Linux
 * carries gcc/clang/icc-specific overrides for likely / unlikely /
 * READ_ONCE / WRITE_ONCE / barrier / etc. The vendored
 * `drivers/virtio/virtio_input.c` reaches few of these directly;
 * most arrive transitively through other headers. Provide just the
 * names the C source's compile-time expansion needs.
 */
#ifndef _LINUX_COMPILER_H
#define _LINUX_COMPILER_H

/* Branch-prediction hints — real Linux uses __builtin_expect. We
 * accept the same wrapper but compile to plain identity (the
 * optimiser doesn't penalise this; clang detects the pattern).
 */
#define likely(x)   (!!(x))
#define unlikely(x) (!!(x))

/* `__init` / `__exit` — section attributes that real Linux drops
 * post-boot. Stubbed to nothing for the linuxkpi shim — no section
 * separation on AREST.
 */
#define __init
#define __exit

/* `__always_inline` — in real Linux this is `inline __attribute__((
 * always_inline))`. clang accepts the bare `__attribute__` form. */
#define __always_inline inline __attribute__((always_inline))

/* `__force` / `__user` / `__kernel` — sparse annotations real Linux
 * uses for address-space checking. AREST has no sparse pass; expand
 * to nothing.
 */
#define __force
#define __user
#define __kernel
#define __iomem
#define __rcu
#define __must_check

/* `READ_ONCE` / `WRITE_ONCE` — volatile load/store helpers. Map to
 * the equivalent volatile dereferences. */
#define READ_ONCE(x)        (*(volatile typeof(x) *)&(x))
#define WRITE_ONCE(x, val)  (*(volatile typeof(x) *)&(x) = (val))

/* Memory barrier — on x86_64 a compiler barrier is sufficient for
 * most kernel paths (the architecture is store-ordered). */
#define barrier() __asm__ __volatile__("" ::: "memory")

/* `__weak` — Linux uses for default-implementation stubs that drivers
 * can override. clang has the same attribute. */
#define __weak __attribute__((weak))

#endif /* _LINUX_COMPILER_H */
