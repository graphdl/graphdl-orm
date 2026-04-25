/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/slab.h` for the AREST linuxkpi shim. Declares the
 * Linux kmalloc/kfree family the vendored
 * `drivers/virtio/virtio_input.c` calls. The implementations live in
 * `crate::linuxkpi::alloc` (Rust) — these are extern decls that the
 * link step resolves to those `#[no_mangle]` exports.
 */
#ifndef _LINUX_SLAB_H
#define _LINUX_SLAB_H

#include <linux/types.h>
#include <linux/gfp.h>

extern void *kmalloc(size_t size, gfp_t gfp);
extern void *kzalloc(size_t size, gfp_t gfp);
extern void  kfree(const void *ptr);

extern void *devm_kzalloc(void *dev, size_t size, gfp_t gfp);
extern void  devm_kfree(void *dev, void *ptr);

#endif /* _LINUX_SLAB_H */
