/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/scatterlist.h` for the AREST linuxkpi shim. The
 * vendored `drivers/virtio/virtio_input.c` builds 1-element
 * scatterlists via `sg_init_one(sg, evtbuf, sizeof(*evtbuf))` for
 * each virtqueue submit. We provide just the struct + the init
 * helper.
 */
#ifndef _LINUX_SCATTERLIST_H
#define _LINUX_SCATTERLIST_H

#include <linux/types.h>

struct scatterlist {
    void   *address;
    size_t  length;
    int     offset;
    int     last;
};

static inline void sg_init_one(struct scatterlist *sg, void *buf, size_t len)
{
    sg->address = buf;
    sg->length  = len;
    sg->offset  = 0;
    sg->last    = 1;
}

#endif /* _LINUX_SCATTERLIST_H */
