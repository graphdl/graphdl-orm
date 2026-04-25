/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `uapi/linux/virtio_ids.h` for the AREST linuxkpi shim.
 *
 * Real Linux's virtio_ids.h is extensively populated. virtio_input.c
 * reaches only `VIRTIO_ID_INPUT`. We provide the standard value
 * (18 — same on every Linux release since virtio-input was added
 * in v3.18, 2014) plus a small selection of related IDs for any
 * future virtio shim that lands here.
 */
#ifndef _UAPI_LINUX_VIRTIO_IDS_H
#define _UAPI_LINUX_VIRTIO_IDS_H

#define VIRTIO_ID_NET          1
#define VIRTIO_ID_BLOCK        2
#define VIRTIO_ID_CONSOLE      3
#define VIRTIO_ID_RNG          4
#define VIRTIO_ID_BALLOON      5
#define VIRTIO_ID_SCSI         8
#define VIRTIO_ID_GPU         16
#define VIRTIO_ID_INPUT       18

#endif /* _UAPI_LINUX_VIRTIO_IDS_H */
