/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/input/mt.h` for the AREST linuxkpi shim. virtio_input.c
 * reaches `input_mt_init_slots` for multitouch devices and dereferences
 * `idev->mt` to detect MT mode (the `idev->mt && type == EV_MSC && code
 * == MSC_TIMESTAMP` check in virtinput_send_status).
 */
#ifndef _LINUX_INPUT_MT_H
#define _LINUX_INPUT_MT_H

#include <linux/input.h>

extern int input_mt_init_slots(struct input_dev *dev, unsigned int num_slots,
                               unsigned int flags);

#endif /* _LINUX_INPUT_MT_H */
