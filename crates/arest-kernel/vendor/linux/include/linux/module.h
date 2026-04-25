/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/module.h` for the AREST linuxkpi shim. Real Linux has
 * full module-loading infrastructure here (kobjects, sysfs, /proc/
 * modules, taint flags, module_param, etc). The vendored
 * `drivers/virtio/virtio_input.c` reaches:
 *   * `MODULE_LICENSE`/`MODULE_DESCRIPTION`/`MODULE_AUTHOR` —
 *     metadata macros, expand to nothing.
 *   * `MODULE_DEVICE_TABLE(virtio, id_table)` — Hotplug helper for
 *     userspace modprobe. AREST has no modprobe; expand to nothing.
 *   * `THIS_MODULE` — pointer to the per-module `struct module`.
 *     Real Linux fills this from a per-translation-unit static; on
 *     AREST we redefine it to NULL via the build.rs `-D` flag.
 *   * `KBUILD_MODNAME` — the module's name string. Defined via
 *     build.rs `-D KBUILD_MODNAME=\"virtio_input\"`.
 *   * `module_init`/`module_exit`/`module_virtio_driver` —
 *     constructor/destructor wiring. Linux's macro variants do
 *     section-magic to register the init function in `.initcall.init`
 *     so the kernel's do_initcalls walker invokes it. AREST has no
 *     initcall walker; we generate a plain `__init virtio_input_
 *     driver_init(void)` extern entry point that `linuxkpi::init()`
 *     calls directly from Rust.
 */
#ifndef _LINUX_MODULE_H
#define _LINUX_MODULE_H

#include <linux/types.h>
#include <linux/compiler.h>

/* Metadata macros — expand to nothing. The strings are dropped at
 * preprocess time. */
#define MODULE_LICENSE(s)
#define MODULE_DESCRIPTION(s)
#define MODULE_AUTHOR(s)
#define MODULE_DEVICE_TABLE(bus, table)

/* `THIS_MODULE` — defined as a void* via build.rs's -D flag, but
 * provide a fallback in case the header is included without the
 * build flag (e.g. IDE-driven linting). */
#ifndef THIS_MODULE
#define THIS_MODULE ((void *)0)
#endif

/* `KBUILD_MODNAME` — same fallback. */
#ifndef KBUILD_MODNAME
#define KBUILD_MODNAME "virtio_input"
#endif

/* module_init / module_exit — Linux's section-magic registers the
 * function in `.initcall.init` for boot-time execution. We replace
 * with plain extern declarations of init/exit hooks. The
 * `module_virtio_driver` macro expands into both halves at once.
 *
 * Naming: virtio_input.c calls `module_virtio_driver(virtio_input_
 * driver)` which expands to:
 *   static int __init virtio_input_driver_init(void) {
 *       return register_virtio_driver(&virtio_input_driver);
 *   }
 *   module_init(virtio_input_driver_init);
 *   ...
 *
 * On AREST we strip `static` (the Rust side calls the symbol via
 * extern "C") and drop the section-attribute trick.
 */
#define module_init(fn)
#define module_exit(fn)

/* module_virtio_driver — single-line wrapper for combined
 * init/exit registration. Expands to:
 *   int virtio_input_driver_init(void) {
 *       return register_virtio_driver(&__virtio_driver);
 *   }
 *   void virtio_input_driver_exit(void) {
 *       unregister_virtio_driver(&__virtio_driver);
 *   }
 *
 * Real Linux's macro is `module_driver(__driver, register_virtio_driver,
 * unregister_virtio_driver)` — we expand the same way.
 */
#define module_virtio_driver(__virtio_driver)                                  \
    int virtio_input_driver_init(void) {                                       \
        return register_virtio_driver(&(__virtio_driver));                     \
    }                                                                          \
    void virtio_input_driver_exit(void) {                                      \
        unregister_virtio_driver(&(__virtio_driver));                          \
    }

/* `EXPORT_SYMBOL` family — Linux uses for the namespace-scoped symbol
 * table that other modules link against. AREST is monolithic; expand
 * to nothing. */
#define EXPORT_SYMBOL(sym)
#define EXPORT_SYMBOL_GPL(sym)

/* `struct module` opaque forward decl — drivers carry pointers but
 * never deref. */
struct module;

#endif /* _LINUX_MODULE_H */
