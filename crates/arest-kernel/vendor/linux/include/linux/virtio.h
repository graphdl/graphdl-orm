/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/virtio.h` for the AREST linuxkpi shim. Layout matches
 * `crate::linuxkpi::virtio::VirtioDevice` / `Virtqueue` exactly.
 *
 * The vendored `drivers/virtio/virtio_input.c` reaches:
 *   * `struct virtio_device` — vdev->priv, vdev->dev, vdev->index,
 *     vdev->config->del_vqs.
 *   * `struct virtqueue` — vq->vdev (back-pointer to device).
 *   * `struct virtio_driver` / `struct virtio_device_id` — driver
 *     descriptor + match table (declared static in the .c file).
 *   * `vq_callback_t` — typedef for the per-queue callback.
 *   * `virtio_find_vqs` — request queues.
 *   * `virtqueue_*` — standalone queue ABI.
 *   * `virtio_has_feature` — feature-bit query.
 *   * `virtio_device_ready` / `virtio_reset_device` — lifecycle.
 *   * `virtio_cread*` / `virtio_cwrite*` — config-space access.
 *
 * Implementations all live in `crate::linuxkpi::virtio`.
 */
#ifndef _LINUX_VIRTIO_H
#define _LINUX_VIRTIO_H

#include <linux/types.h>
#include <linux/device.h>

/* Forward decls — the Rust mirror does the same. */
struct virtio_device;
struct virtqueue;

/* `vq_callback_t` — type alias for the per-queue callback signature.
 * virtio_input.c declares an array of these. */
typedef void vq_callback_t(struct virtqueue *vq);

/* `struct virtio_device_id` — vendor/device-id pair. UAPI but
 * referenced from the kernel-side driver descriptor too.
 */
struct virtio_device_id {
    __u32 device;
    __u32 vendor;
};

/* `struct virtio_config_ops` — bus vtable. virtio_input.c reaches
 * only `del_vqs`; we declare a minimal subset for layout parity.
 */
struct virtio_config_ops {
    void (*del_vqs)(struct virtio_device *vdev);
};

/* `struct virtio_device` — Linux's per-device handle. Layout MUST
 * match crate::linuxkpi::virtio::VirtioDevice exactly.
 */
struct virtio_device {
    int                              index;
    void                            *priv;
    struct device                    dev;
    struct virtio_config_ops        *config;
    struct virtio_device_id          id;
};

/* `struct virtqueue` — per-queue handle. Layout MUST match
 * crate::linuxkpi::virtio::Virtqueue exactly.
 */
struct virtqueue {
    struct virtio_device   *vdev;
    __u32                   index;
    void                  (*callback)(struct virtqueue *vq);
    void                   *priv;
};

/* `struct virtio_driver` — driver descriptor. Layout MUST match
 * crate::linuxkpi::driver::VirtioDriver exactly. The `driver` field
 * is the embedded `struct device_driver` base class.
 */
struct device_driver {
    const char     *name;
    void           *owner;
};

struct virtio_driver {
    struct device_driver               driver;
    const struct virtio_device_id     *id_table;
    const unsigned int                *feature_table;
    unsigned int                       feature_table_size;
    const unsigned int                *feature_table_legacy;
    unsigned int                       feature_table_size_legacy;
    int   (*probe)(struct virtio_device *dev);
    void  (*scan)(struct virtio_device *dev);
    void  (*remove)(struct virtio_device *dev);
    void  (*config_changed)(struct virtio_device *dev);
    int   (*freeze)(struct virtio_device *dev);
    int   (*restore)(struct virtio_device *dev);
};

/* Driver registration. */
extern int  register_virtio_driver(struct virtio_driver *drv);
extern void unregister_virtio_driver(struct virtio_driver *drv);

/* Virtqueue ABI. */
extern int   virtio_find_vqs(struct virtio_device *vdev, unsigned int nvqs,
                             struct virtqueue *vqs[],
                             vq_callback_t *callbacks[],
                             const char * const names[],
                             void *desc);
extern int   virtqueue_add_inbuf(struct virtqueue *vq, void *sg,
                                 unsigned int num, void *data,
                                 int gfp);
extern int   virtqueue_add_outbuf(struct virtqueue *vq, void *sg,
                                  unsigned int num, void *data,
                                  int gfp);
extern void *virtqueue_get_buf(struct virtqueue *vq, unsigned int *len);
extern bool  virtqueue_kick(struct virtqueue *vq);
extern unsigned int virtqueue_get_vring_size(struct virtqueue *vq);
extern void *virtqueue_detach_unused_buf(struct virtqueue *vq);

/* Feature query / lifecycle. */
extern bool virtio_has_feature(struct virtio_device *vdev, unsigned int fbit);
extern void virtio_device_ready(struct virtio_device *vdev);
extern void virtio_reset_device(struct virtio_device *vdev);

/* Standard virtio feature bit values. virtio_input.c checks
 * VIRTIO_F_VERSION_1 at probe entry. */
#define VIRTIO_F_VERSION_1       32
#define VIRTIO_DEV_ANY_ID        0xffffffff

#endif /* _LINUX_VIRTIO_H */
