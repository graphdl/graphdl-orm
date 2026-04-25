/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/input.h` for the AREST linuxkpi shim. Layout matches
 * `crate::linuxkpi::input::InputDev` exactly — extending here
 * requires extending there.
 *
 * Bitmap sizes match the EV_/KEY_/REL_/ABS_ etc constants below
 * via BITS_TO_LONGS. virtio_input.c uses __set_bit/test_bit against
 * these arrays during probe.
 */
#ifndef _LINUX_INPUT_H
#define _LINUX_INPUT_H

#include <linux/types.h>
#include <linux/bitops.h>
#include <linux/device.h>
#include <linux/kernel.h>

/* Event types — Linux's input_event_code.h. virtio_input.c reaches:
 *   EV_SYN, EV_KEY, EV_REL, EV_ABS, EV_MSC, EV_SW, EV_LED, EV_SND,
 *   EV_REP. */
#define EV_SYN  0x00
#define EV_KEY  0x01
#define EV_REL  0x02
#define EV_ABS  0x03
#define EV_MSC  0x04
#define EV_SW   0x05
#define EV_LED  0x11
#define EV_SND  0x12
#define EV_REP  0x14
#define EV_FF   0x15
#define EV_PWR  0x16
#define EV_CNT  0x20

/* Code-space sizes (BITS_TO_LONGS dividends). */
#define KEY_CNT          0x300
#define REL_CNT          0x10
#define ABS_CNT          0x40
#define MSC_CNT          0x08
#define SW_CNT           0x10
#define LED_CNT          0x10
#define SND_CNT          0x08
#define FF_CNT           0x80
#define INPUT_PROP_CNT   0x20

/* Specific codes virtio_input.c references. */
#define MSC_TIMESTAMP    0x05
#define ABS_MT_SLOT      0x2f
#define BUS_VIRTUAL      0x06

/* `struct input_id` — bus/vendor/product/version fingerprint. */
struct input_id {
    u16 bustype;
    u16 vendor;
    u16 product;
    u16 version;
};

/* `struct input_absinfo` — per-axis ABS calibration. */
struct input_absinfo {
    s32 value;
    s32 minimum;
    s32 maximum;
    s32 fuzz;
    s32 flat;
    s32 resolution;
};

/* Forward decls so input_dev's `event` callback can typecheck. */
struct input_dev;

/* `struct input_dev` — central per-device state. Layout MUST match
 * crate::linuxkpi::input::InputDev exactly. */
struct input_dev {
    const char *name;
    const char *phys;
    const char *uniq;
    struct input_id id;

    unsigned long propbit[BITS_TO_LONGS(INPUT_PROP_CNT)];
    unsigned long evbit  [BITS_TO_LONGS(EV_CNT)];
    unsigned long keybit [BITS_TO_LONGS(KEY_CNT)];
    unsigned long relbit [BITS_TO_LONGS(REL_CNT)];
    unsigned long absbit [BITS_TO_LONGS(ABS_CNT)];
    unsigned long mscbit [BITS_TO_LONGS(MSC_CNT)];
    unsigned long ledbit [BITS_TO_LONGS(LED_CNT)];
    unsigned long sndbit [BITS_TO_LONGS(SND_CNT)];
    unsigned long ffbit  [BITS_TO_LONGS(FF_CNT)];
    unsigned long swbit  [BITS_TO_LONGS(SW_CNT)];

    struct input_absinfo *absinfo;

    void *mt;

    struct device dev;

    int (*event)(struct input_dev *dev, unsigned int type,
                 unsigned int code, int value);

    void *driver_data;
    int   users;
};

/* Allocation / registration. */
extern struct input_dev *input_allocate_device(void);
extern void  input_free_device(struct input_dev *dev);
extern int   input_register_device(struct input_dev *dev);
extern void  input_unregister_device(struct input_dev *dev);

/* Event push. */
extern void input_event(struct input_dev *dev, unsigned int type,
                        unsigned int code, int value);
extern void input_sync(struct input_dev *dev);

/* drvdata helpers. */
extern void  input_set_drvdata(struct input_dev *dev, void *data);
extern void *input_get_drvdata(struct input_dev *dev);

/* ABS axis configuration. */
extern void input_set_abs_params(struct input_dev *dev, unsigned int axis,
                                 int min, int max, int fuzz, int flat);
extern void input_abs_set_res(struct input_dev *dev, unsigned int axis,
                              int res);
extern int  input_abs_get_max(struct input_dev *dev, unsigned int axis);

#endif /* _LINUX_INPUT_H */
