/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/bitops.h` for the AREST linuxkpi shim. virtio_input.c
 * uses `__set_bit` / `test_bit` against the per-device evbit/keybit/
 * relbit/absbit/etc bitmaps in `struct input_dev`.
 *
 * Implementation notes
 * --------------------
 * Linux's real __set_bit is non-atomic; `set_bit` is atomic. We
 * provide both as the same non-atomic implementation — single-CPU
 * AREST has no atomic distinction.
 *
 * Bits are stored in arrays of `unsigned long` (8 bytes on x86_64).
 * Index N lives in word N/64, bit N%64. Same convention real Linux
 * uses; the InputDev Rust mirror stores `[u64; ...]` arrays sized
 * to match.
 */
#ifndef _LINUX_BITOPS_H
#define _LINUX_BITOPS_H

#include <linux/types.h>

#define BITS_PER_LONG       64
#define BIT(n)              (1UL << (n))
#define BIT_MASK(nr)        (1UL << ((nr) % BITS_PER_LONG))
#define BIT_WORD(nr)        ((nr) / BITS_PER_LONG)
#define BITS_TO_LONGS(nr)   (((nr) + BITS_PER_LONG - 1) / BITS_PER_LONG)

static inline void __set_bit(int nr, volatile unsigned long *addr)
{
    addr[BIT_WORD(nr)] |= BIT_MASK(nr);
}

static inline void __clear_bit(int nr, volatile unsigned long *addr)
{
    addr[BIT_WORD(nr)] &= ~BIT_MASK(nr);
}

static inline int test_bit(int nr, const volatile unsigned long *addr)
{
    return (addr[BIT_WORD(nr)] >> (nr % BITS_PER_LONG)) & 1UL;
}

/* Atomic variants — single-CPU AREST, same as the non-atomic. */
#define set_bit(nr, addr)    __set_bit((nr), (addr))
#define clear_bit(nr, addr)  __clear_bit((nr), (addr))

#endif /* _LINUX_BITOPS_H */
