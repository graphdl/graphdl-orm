/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/spinlock.h` for the AREST linuxkpi shim. Single-CPU
 * AREST has no scheduler — the lock primitives expand to no-ops at
 * the C level, with `flags` being a token int that holds the saved
 * IRQ-enable state (we don't toggle IRQs here, but the macros need
 * to expand to syntactically-valid expressions).
 *
 * The vendored `drivers/virtio/virtio_input.c` uses spin_lock_irqsave
 * extensively to guard `vi->ready` and the virtqueue pointer state
 * against IRQ-context callbacks. On AREST, the IRQ that would otherwise
 * race the bottom-half is handled inside the linuxkpi virtio shim,
 * which is itself single-threaded (per-tick drain from launcher), so
 * the lock is a formality.
 */
#ifndef _LINUX_SPINLOCK_H
#define _LINUX_SPINLOCK_H

#include <linux/types.h>

typedef struct spinlock {
    int dummy;
} spinlock_t;

#define spin_lock_init(lock)               do { (lock)->dummy = 0; } while (0)
#define spin_lock(lock)                    ((void)(lock))
#define spin_unlock(lock)                  ((void)(lock))
#define spin_lock_irqsave(lock, flags)     do { (void)(lock); (flags) = 0; } while (0)
#define spin_unlock_irqrestore(lock, flags) do { (void)(lock); (void)(flags); } while (0)
#define spin_lock_irq(lock)                ((void)(lock))
#define spin_unlock_irq(lock)              ((void)(lock))

#endif /* _LINUX_SPINLOCK_H */
