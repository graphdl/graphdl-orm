/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/kernel.h` for the AREST linuxkpi shim. Real Linux's
 * kernel.h is enormous (kernel printing, integer math, container_of,
 * BUG_ON, container helpers...). The vendored
 * `drivers/virtio/virtio_input.c` uses just a few of those — provide
 * minimal expansions.
 */
#ifndef _LINUX_KERNEL_H
#define _LINUX_KERNEL_H

#include <linux/types.h>
#include <linux/compiler.h>
#include <linux/errno.h>

/* Compile-time array length. Same expansion every Linux subsys uses. */
#define ARRAY_SIZE(arr) (sizeof(arr) / sizeof((arr)[0]))

/* `min` / `max` — type-generic via __auto_type (GCC 4.9+, clang). */
#ifndef min
#define min(x, y) ({                                    \
    __auto_type _min1 = (x);                            \
    __auto_type _min2 = (y);                            \
    _min1 < _min2 ? _min1 : _min2; })
#endif
#ifndef max
#define max(x, y) ({                                    \
    __auto_type _max1 = (x);                            \
    __auto_type _max2 = (y);                            \
    _max1 > _max2 ? _max1 : _max2; })
#endif

/* Linux's container_of — given a pointer to a member, find the
 * enclosing struct. Pure pointer arithmetic.
 */
#define container_of(ptr, type, member) ({              \
    void *__mptr = (void *)(ptr);                       \
    ((type *)(__mptr - offsetof(type, member))); })

/* offsetof — clang's builtin matches GCC's. */
#ifndef offsetof
#define offsetof(type, member) __builtin_offsetof(type, member)
#endif

/* Linux uses `void *` for snprintf-style printer. We provide a
 * trivial declaration so virtio_input.c's snprintf call links —
 * real implementation lives in the (Rust-side) shim if needed.
 * For the foundation slice the call resolves at link time but is
 * never invoked (probe never fires).
 */
extern int snprintf(char *buf, size_t size, const char *fmt, ...);

/* `printk` — Linux's kernel print. Stubbed to a no-op for the slice.
 * Real wiring would route to crate::println!. */
#define KERN_INFO    ""
#define KERN_DEBUG   ""
#define KERN_ERR     ""
#define KERN_WARNING ""
#define printk(...)  ((void)0)

/* BUG_ON / WARN_ON — runtime assertions. No-op for the slice. */
#define BUG_ON(cond)  ((void)(cond))
#define WARN_ON(cond) ((void)(cond))

#endif /* _LINUX_KERNEL_H */
