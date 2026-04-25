// crates/arest-kernel/src/linuxkpi/irq.rs
//
// Linux IRQ registration shim — `request_irq`, `free_irq`,
// `synchronize_irq`. The vendored `virtio_input.c` does NOT call
// these directly (its IRQ wiring is hidden inside the virtio-pci /
// virtio-mmio transport's `find_vqs` path), but Linux's virtio bus
// expects the entry points to exist for the indirect callers — and
// the broader linuxkpi surface needs them for any future driver
// (touchscreen, WiFi) we link in.
//
// What we model
// -------------
// Linux IRQ registration:
//   * `request_irq(irq, handler, flags, name, dev_id)` installs an
//     interrupt handler on a Linux virtual IRQ number. The handler
//     signature is `irqreturn_t (*)(int irq, void *dev_id)`.
//   * `free_irq(irq, dev_id)` un-installs it.
//   * `synchronize_irq(irq)` waits for any in-flight handler on `irq`
//     to complete. On a single-CPU kernel with no preemption, this is
//     a memory fence — every IRQ has by definition completed before
//     the call site reached this instruction.
//
// AREST already has IDT vector slots wired in
// `arch::uefi::interrupts` (vectors 32 = PIT timer, 33 = PS/2
// keyboard, 34..47 stub, 48..255 spurious). Mapping a Linux IRQ
// number to an IDT vector is a static offset on the foundation slice
// — Linux IRQ N → IDT vector 32 + N for hardware IRQs 0..15. The IRQ
// number space above 16 is reserved for "virtual" IRQs (MSI / MSI-X)
// which the virtio-pci transport synthesises at probe time; we don't
// support those in this slice.
//
// Storage
// -------
// A small static table of (irq, handler, dev_id) triples. The IDT
// vectors themselves stay owned by `arch::uefi::interrupts` — we
// don't try to splice into the IDT here, because that would race
// the existing PIT/PS-2 handlers. Instead, the future bus walker in
// #459b is responsible for plumbing IDT-vector-receive into a
// dispatch loop that calls into our table. On the foundation slice
// the table is populated but never read.

use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{compiler_fence, Ordering};
use spin::{Mutex, Once};

/// Linux's `irqreturn_t` enum. `IRQ_HANDLED = 1` means the handler
/// claimed the line; `IRQ_NONE = 0` means it didn't recognise the
/// IRQ as its own (used in shared-IRQ chains). We export the raw
/// constants so the C side resolves them by header value.
pub const IRQ_NONE: c_int = 0;
pub const IRQ_HANDLED: c_int = 1;
pub const IRQ_WAKE_THREAD: c_int = 2;

/// Type alias for the Linux IRQ handler function pointer.
pub type IrqHandlerFn = unsafe extern "C" fn(c_int, *mut c_void) -> c_int;

/// Registered IRQ entry. Pure bookkeeping; the IDT side is owned
/// by `arch::uefi::interrupts`.
///
/// The `*mut c_void` `dev_id` makes this struct non-Send by default,
/// but the pointer is opaque from this module's perspective — it's
/// only ever handed back to the registered handler verbatim. Single-
/// threaded kernel — no concurrent access; the unsafe Send/Sync
/// impls only exist to satisfy the static Once's Sync bound.
#[repr(C)]
struct Registration {
    irq: c_int,
    handler: IrqHandlerFn,
    dev_id: *mut c_void,
}

unsafe impl Send for Registration {}
unsafe impl Sync for Registration {}

static IRQS: Once<Mutex<Vec<Registration>>> = Once::new();

pub fn init() {
    IRQS.call_once(|| Mutex::new(Vec::new()));
}

/// `request_irq(irq, handler, flags, name, dev_id)` — register a
/// Linux-style IRQ handler. Returns 0 on success, -ENOMEM (-12) on
/// allocation failure (we don't OOM under realistic registration
/// rates, but the contract requires the return code to exist).
///
/// `flags` and `name` are accepted but ignored on the foundation
/// slice — `flags` is shared / edge-trigger metadata that's
/// meaningful only when we're driving the controller directly, and
/// `name` is for /proc/interrupts which AREST doesn't expose.
#[no_mangle]
pub extern "C" fn request_irq(
    irq: c_int,
    handler: IrqHandlerFn,
    _flags: u64,
    _name: *const c_char,
    dev_id: *mut c_void,
) -> c_int {
    if let Some(reg) = IRQS.get() {
        reg.lock().push(Registration {
            irq,
            handler,
            dev_id,
        });
    }
    0
}

/// `free_irq(irq, dev_id)` — un-register the handler matching the
/// (irq, dev_id) pair. Returns the dev_id pointer (Linux's
/// `void *free_irq(...)` convention).
#[no_mangle]
pub extern "C" fn free_irq(irq: c_int, dev_id: *mut c_void) -> *mut c_void {
    if let Some(reg) = IRQS.get() {
        reg.lock()
            .retain(|r| !(r.irq == irq && r.dev_id == dev_id));
    }
    dev_id
}

/// `synchronize_irq(irq)` — wait for any in-flight handler on `irq`
/// to complete. On single-CPU AREST with no preemption, no IRQ can
/// be in-flight relative to a non-IRQ caller (the CPU is by
/// definition NOT executing both the call site and an IRQ handler
/// simultaneously). The only thing we owe the caller is a memory
/// barrier so any state the IRQ touched is visible here.
///
/// `compiler_fence(SeqCst)` is sufficient on x86_64 because the
/// architecture is store-ordered — the CPU never reorders stores
/// across a cli/sti boundary, which is implicit at IRQ entry/exit.
#[no_mangle]
pub extern "C" fn synchronize_irq(_irq: c_int) {
    compiler_fence(Ordering::SeqCst);
}

/// `disable_irq(irq)` — Linux entry that masks an IRQ at the
/// controller. We don't expose individual mask bits at the linuxkpi
/// layer (the PIC mask state is owned by `arch::uefi::interrupts`),
/// so this is a defensive no-op. A driver that hard-depends on
/// disable_irq actually masking the line will misbehave; for the
/// foundation slice nothing reaches it.
#[no_mangle]
pub extern "C" fn disable_irq(_irq: c_int) {}

/// `enable_irq(irq)` — pair with `disable_irq`. Same no-op story.
#[no_mangle]
pub extern "C" fn enable_irq(_irq: c_int) {}
