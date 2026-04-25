/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/device.h` for the AREST linuxkpi shim. Layout matches
 * `crate::linuxkpi::device::Device` exactly — extending here requires
 * extending there.
 *
 * `struct device` is enormous in real Linux (~700 bytes). We model
 * only the fields virtio_input.c reaches: parent, driver_data, bus,
 * release.
 */
#ifndef _LINUX_DEVICE_H
#define _LINUX_DEVICE_H

#include <linux/types.h>

struct device {
    struct device *parent;
    void          *driver_data;
    void          *bus;
    void         (*release)(struct device *dev);
};

extern int  device_register(struct device *dev);
extern void device_unregister(struct device *dev);
extern void dev_set_drvdata(struct device *dev, void *data);
extern void *dev_get_drvdata(struct device *dev);
extern void put_device(struct device *dev);
extern struct device *get_device(struct device *dev);

#endif /* _LINUX_DEVICE_H */
