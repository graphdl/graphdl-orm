// crates/arest-kernel/src/arch/x86_64/mod.rs
//
// x86_64 arch impl. Houses the pieces of the kernel that are tied to
// x86_64 silicon — the 16550 UART, GDT/TSS, IDT + PIC, OffsetPageTable
// construction, and the idle loop. The shared kernel body in `main.rs`
// reaches these through the `arch::` facade so the aarch64 impl can
// slot in underneath without touching the body.
//
// Today the BIOS entry (`bootloader_api` → `kernel_main`) is the only
// caller. Step 4 of the UEFI pivot (#344) wires the UEFI entry to the
// same facade after ExitBootServices.

pub mod gdt;
pub mod interrupts;
pub mod memory;
pub mod serial;

// `_print` is the function the `print!` / `println!` macros at the
// crate root call. Expose it at `arch::_print` so the macros can
// route target-agnostically without knowing which arch module
// supplies the UART driver.
pub use serial::_print;

/// Initialise the architecture's console. Called once at boot before
/// any `println!`. On the BIOS path the 16550 `LazyUart` lazy-init-
/// ialises on first `SERIAL.lock()`, so this is effectively a no-op
/// — kept as an explicit entry point so the UEFI path can install its
/// ConOut-backed console here (step 3 of #344) without the kernel
/// body caring.
pub fn init_console() {
    // Intentionally empty — see docstring.
}

/// Bring up descriptor tables + interrupts. GDT + TSS first (so the
/// TSS entry referenced by the IDT's double-fault gate exists), then
/// IDT, then the 8259 PIC remap + `sti`. Must run before any IRQ can
/// fire.
pub fn init_gdt_and_interrupts() {
    gdt::init();
    interrupts::init_idt();
    interrupts::init_pic();
}

/// Initialise the memory subsystem from the bootloader's `BootInfo`.
/// Builds the `OffsetPageTable`, carves the DMA pool, and stands up
/// the boot-time frame allocator. Returns the bootloader-mapped
/// physical-memory offset so callers (notably `virtio::init_offset`)
/// don't need to reach into the BIOS-shaped `BootInfo` themselves.
pub fn init_memory(boot_info: &'static bootloader_api::BootInfo) -> u64 {
    memory::init(boot_info);
    boot_info
        .physical_memory_offset
        .into_option()
        .expect("bootloader did not supply physical_memory_offset")
}

/// Fire a software breakpoint (`int3` on x86). Used by the boot banner
/// to prove the IDT routes CPU exceptions back into the Rust handler
/// chain cleanly.
pub fn breakpoint() {
    ::x86_64::instructions::interrupts::int3();
}

/// Drive the kernel's idle loop. Busy-polls `net::poll()` so smoltcp
/// can advance DHCP, TCP retransmit, and HTTP dispatch without a
/// dedicated periodic IRQ.
///
/// Trade-off: 100 % CPU when idle, because a naive `hlt` here only
/// wakes on a keyboard IRQ (the sole IRQ currently unmasked in the
/// PIC) — which never fires in the E2E smoke harness, so DHCP stalls
/// before it can request a lease from QEMU's SLiRP and `curl` times
/// out at the host (observed pre-fix, #268). Once a periodic timer
/// IRQ (#180 follow-up) or a PCI-line virtio IRQ lands, this can go
/// back to `hlt`-then-poll.
///
/// Interrupts stay enabled throughout, so keyboard / exception ISRs
/// still fire and return back into the loop.
pub fn halt_forever() -> ! {
    loop {
        crate::net::poll();
        core::hint::spin_loop();
    }
}
