/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/virtio_config.h` for the AREST linuxkpi shim. virtio
 * drivers use the `virtio_cread*` / `virtio_cwrite*` macros to read
 * per-device config-space fields. The vendored
 * `drivers/virtio/virtio_input.c` uses:
 *   * virtio_cread_le(vdev, struct virtio_input_config, FIELD, &val)
 *   * virtio_cwrite_le(vdev, struct virtio_input_config, FIELD, &val)
 *   * virtio_cread_bytes(vdev, offset, buf, len)
 *
 * The macros expand to per-field offsetof() + size dispatch into
 * virtio_cread{8,16,32,64} / virtio_cwrite{8,16,32,64}. Same pattern
 * real Linux uses.
 */
#ifndef _LINUX_VIRTIO_CONFIG_H
#define _LINUX_VIRTIO_CONFIG_H

#include <linux/types.h>
#include <linux/virtio.h>
#include <linux/byteorder.h>
#include <linux/kernel.h>

/* Raw per-width config-space accessors (extern — Rust shim). */
extern u8   virtio_cread8(struct virtio_device *vdev, u32 offset);
extern u16  virtio_cread16(struct virtio_device *vdev, u32 offset);
extern u32  virtio_cread32(struct virtio_device *vdev, u32 offset);
extern u64  virtio_cread64(struct virtio_device *vdev, u32 offset);

extern void virtio_cwrite8(struct virtio_device *vdev, u32 offset, u8 val);
extern void virtio_cwrite16(struct virtio_device *vdev, u32 offset, u16 val);
extern void virtio_cwrite32(struct virtio_device *vdev, u32 offset, u32 val);
extern void virtio_cwrite64(struct virtio_device *vdev, u32 offset, u64 val);

extern void virtio_cread_bytes(struct virtio_device *vdev, u32 offset,
                               void *buf, size_t len);
extern void virtio_cwrite_bytes(struct virtio_device *vdev, u32 offset,
                                const void *buf, size_t len);

/* virtio_cread_le / virtio_cwrite_le — type-generic field readers.
 * Real Linux uses _Generic-driven dispatch on the field type; we
 * use sizeof() dispatch which is portable across both clang and gcc.
 *
 * The destination is dereferenced by the caller — `&val` patterns
 * everywhere — so we write into *valp.
 */
#define virtio_cread_le(vdev, struct_type, field, valp)                       \
    do {                                                                       \
        u32 _off = (u32)offsetof(struct_type, field);                         \
        switch (sizeof(*(valp))) {                                            \
            case 1: *(u8  *)(valp) = virtio_cread8 ((vdev), _off); break;     \
            case 2: *(u16 *)(valp) = virtio_cread16((vdev), _off); break;     \
            case 4: *(u32 *)(valp) = virtio_cread32((vdev), _off); break;     \
            case 8: *(u64 *)(valp) = virtio_cread64((vdev), _off); break;     \
            default: break;                                                   \
        }                                                                      \
    } while (0)

#define virtio_cwrite_le(vdev, struct_type, field, valp)                      \
    do {                                                                       \
        u32 _off = (u32)offsetof(struct_type, field);                         \
        switch (sizeof(*(valp))) {                                            \
            case 1: virtio_cwrite8 ((vdev), _off, *(u8  *)(valp)); break;     \
            case 2: virtio_cwrite16((vdev), _off, *(u16 *)(valp)); break;     \
            case 4: virtio_cwrite32((vdev), _off, *(u32 *)(valp)); break;     \
            case 8: virtio_cwrite64((vdev), _off, *(u64 *)(valp)); break;     \
            default: break;                                                   \
        }                                                                      \
    } while (0)

#endif /* _LINUX_VIRTIO_CONFIG_H */
