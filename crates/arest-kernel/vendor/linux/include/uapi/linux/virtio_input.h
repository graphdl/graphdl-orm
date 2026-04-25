/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `uapi/linux/virtio_input.h` for the AREST linuxkpi shim. This
 * is the wire-protocol header — selectors / config-space layout / the
 * input-event packet format. Real Linux's version is verbatim
 * authoritative; we mirror it here because any change would break
 * the protocol contract.
 */
#ifndef _UAPI_LINUX_VIRTIO_INPUT_H
#define _UAPI_LINUX_VIRTIO_INPUT_H

#include <linux/types.h>

/* virtio_input_config select values — `select` field of the per-
 * device config-space struct. */
#define VIRTIO_INPUT_CFG_UNSET      0x00
#define VIRTIO_INPUT_CFG_ID_NAME    0x01
#define VIRTIO_INPUT_CFG_ID_SERIAL  0x02
#define VIRTIO_INPUT_CFG_ID_DEVIDS  0x03
#define VIRTIO_INPUT_CFG_PROP_BITS  0x10
#define VIRTIO_INPUT_CFG_EV_BITS    0x11
#define VIRTIO_INPUT_CFG_ABS_INFO   0x12

/* `struct virtio_input_absinfo` — wire format for ABS axis
 * calibration. Same shape as the kernel-internal `input_absinfo`
 * minus the `value` field. */
struct virtio_input_absinfo {
    __le32 min;
    __le32 max;
    __le32 fuzz;
    __le32 flat;
    __le32 res;
};

/* `struct virtio_input_devids` — bus/vendor/product/version. */
struct virtio_input_devids {
    __le16 bustype;
    __le16 vendor;
    __le16 product;
    __le16 version;
};

/* `struct virtio_input_config` — device's exposed config space.
 * The `u` union carries select-dependent payload variants; layout
 * MUST be byte-identical to real Linux's so the offsetof() lookups
 * in virtio_input.c read the right bytes.
 */
struct virtio_input_config {
    __u8 select;
    __u8 subsel;
    __u8 size;
    __u8 reserved[5];
    union {
        char string[128];
        __u8 bitmap[128];
        struct virtio_input_absinfo abs;
        struct virtio_input_devids ids;
    } u;
};

/* `struct virtio_input_event` — wire format for one event packet. */
struct virtio_input_event {
    __le16 type;
    __le16 code;
    __le32 value;
};

#endif /* _UAPI_LINUX_VIRTIO_INPUT_H */
