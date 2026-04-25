/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/gfp.h` for the AREST linuxkpi shim. Defines the
 * `gfp_t` enum values virtio_input.c passes to kmalloc/kzalloc.
 * The Rust shim ignores `gfp` entirely — single-threaded kernel
 * with no scheduler means no sleep-vs-atomic distinction.
 */
#ifndef _LINUX_GFP_H
#define _LINUX_GFP_H

#include <linux/types.h>

/* Standard GFP flag values from real Linux. The Rust shim accepts
 * any unsigned int.
 */
#define GFP_KERNEL  0x000020c0u
#define GFP_ATOMIC  0x00000820u
#define GFP_NOWAIT  0x00000800u
#define GFP_DMA     0x00000001u

#endif /* _LINUX_GFP_H */
