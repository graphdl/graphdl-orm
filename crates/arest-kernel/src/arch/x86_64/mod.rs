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

pub mod serial;

// `_print` is the function the `print!` / `println!` macros at the
// crate root call. Expose it at `arch::_print` so the macros can
// route target-agnostically without knowing which arch module
// supplies the UART driver.
pub use serial::_print;
