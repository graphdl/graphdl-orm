// crates/arest-kernel/src/linuxkpi/mod.rs
//
// linuxkpi — FreeBSD-style Linux kernel API shim foundation (#460,
// Track AAAA / #459a). Goal: let *unmodified* Linux kernel C drivers
// link against AREST primitives, the same way FreeBSD's
// `sys/compat/linuxkpi/` reuses the iwlwifi WiFi stack and DRM/KMS
// graphics drivers verbatim. This is the foundation slice — only
// enough kernel API surface for `drivers/virtio/virtio_input.c` (the
// smallest virtio device-class driver) to compile + link cleanly.
//
// Architecture rationale
// ----------------------
// AREST today writes every driver from scratch: virtio-net, virtio-blk,
// virtio-gpu all live under `crates/arest-kernel/src/` as hand-rolled
// pure-Rust drivers built atop rcore-os/virtio-drivers. That doesn't
// scale: real hardware needs Broadcom WiFi (no upstream Rust port),
// touchscreen panels (Goodix / FocalTech / etc — Linux drivers only),
// MIPI DSI panels, USB HID, etc. Rewriting each is months of work per
// driver. Linking unmodified Linux drivers against a thin compat
// shim is the proven approach (FreeBSD has been doing it since 2014).
//
// What this slice ships
// ---------------------
// 8 sub-modules, each minimal but real (no `unimplemented!()` body
// in the public API — every entry returns a sound default or maps
// to existing AREST infrastructure):
//
//   alloc       — kmalloc / kfree / kzalloc / devm_kzalloc / devm_kfree
//   device      — struct device + register / unregister + drvdata
//   driver      — struct driver_register + bus_type + match tables
//   irq         — request_irq / free_irq / synchronize_irq
//   workqueue   — INIT_WORK / queue_work / cancel_work_sync
//   io          — ioremap / iounmap / read{b,w,l,q} / write{b,w,l,q}
//   input       — input_register_device / input_event / input_sync
//   virtio      — virtqueue / virtio_device / virtio_find_vqs
//
// The C-side stub headers under `crates/arest-kernel/vendor/linux/
// include/linux/*.h` declare the prototypes that match these Rust
// `extern "C"` exports. The vendored `drivers/virtio/virtio_input.c`
// `#include`s those headers, so the C compiler resolves every symbol
// at link time.
//
// Gating
// ------
// The whole subsystem is opt-in behind the `linuxkpi` cargo feature
// (see `Cargo.toml`). Default kernel builds skip the C compile and
// every Rust shim module — the .efi footprint and license story
// (AGPL-3.0-or-later only by default; --features linuxkpi inherits
// the GPL-2.0 of the vendored Linux source per FSF compatibility) are
// unchanged. Mirrors VVV's #456 doom-feature pattern in
// `src/main.rs`.
//
// Lifecycle
// ---------
// `init()` is called once from main.rs after `arch::init_*` has
// brought the kernel up. It walks each sub-module's static-initializer
// requirement and then calls the C-side `module_virtio_driver` macro's
// `__module_init` symbol via `virtio_input_driver_init`. The shim's
// goal at this slice is solely *clean linkage* — actual virtio-input
// device discovery + event-queue wiring is #459b scope.

#![allow(dead_code)]

pub mod alloc;
pub mod device;
pub mod driver;
pub mod input;
pub mod io;
pub mod irq;
pub mod virtio;
pub mod workqueue;

/// One-shot boot-time initialiser. Brings every linuxkpi sub-system
/// online in dependency order (alloc first because devm_* rides on
/// it; device + driver before input because input registers a
/// per-device match; workqueue last because nothing else queues yet).
///
/// Then calls into the C-side virtio-input module init function. On
/// this foundation slice the C side is link-resolved only — real
/// virtio-input device probe + event flow is #459b. The init call is
/// here so a successful `linuxkpi::init()` proves all the C-side
/// symbol references resolve at runtime, not just compile-time.
///
/// Idempotent — each sub-module's init is `Once`-guarded so a second
/// call is a no-op.
pub fn init() {
    alloc::init();
    device::init();
    driver::init();
    irq::init();
    workqueue::init();
    io::init();
    input::init();
    virtio::init();

    // Hand off to the vendored C driver's module-init thunk. The
    // `virtio_input_driver_init` symbol is emitted by the
    // `module_virtio_driver(virtio_input_driver)` macro in
    // `vendor/linux/drivers/virtio/virtio_input.c`. On the foundation
    // slice this only proves the symbol resolves at link time — the
    // virtio bus itself isn't wired to deliver a device-add event yet
    // (that's #459b).
    //
    // Gated on `linuxkpi_c_linked` — a custom cfg the build script
    // sets only when the cc::Build invocation actually produced
    // libvirtio_input.a. On hosts without a C cross-compiler in PATH
    // (Windows host without clang or VS Developer PowerShell, etc),
    // the build script emits a warning and skips the cc step; this
    // call is then elided so the link doesn't fail on an unresolved
    // extern. The Rust shim modules still compile + register; only
    // the C-driver bring-up is skipped on degraded hosts.
    //
    // SAFETY: zero-arg extern "C" fn with no kernel-state precondition
    // beyond what we already initialised above. The Linux thunk just
    // calls `register_virtio_driver(&virtio_input_driver)`, which in
    // our shim is a static-table append (see `driver.rs`).
    #[cfg(linuxkpi_c_linked)]
    unsafe {
        extern "C" {
            fn virtio_input_driver_init() -> core::ffi::c_int;
        }
        let _ = virtio_input_driver_init();
    }
}

/// Per-frame tick called from the launcher super-loop (see
/// `src/ui_apps/launcher.rs`). Drains the workqueue ring so any
/// `queue_work` calls a Linux driver makes from IRQ / callback
/// context get to run on the foreground. Cheap when idle.
pub fn tick() {
    workqueue::drain();
}
