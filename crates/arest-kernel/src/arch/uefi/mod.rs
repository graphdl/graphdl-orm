// crates/arest-kernel/src/arch/uefi/mod.rs
//
// UEFI arch arm (#344 step 3). Supplies the minimum the shared
// `print!` / `println!` macros need so kernel code that today runs
// only on the BIOS path can produce output under UEFI without
// touching the call sites.
//
// What's implemented:
//   * `_print(args)` — formats into a `String`, transcodes to UCS-2,
//     and pushes through the firmware's ConOut Simple Text Output
//     Protocol via `uefi::system::with_stdout`.
//   * `init_console()` — no-op. ConOut is already configured by the
//     firmware before our entry runs; we just have to be willing to
//     call it.
//
// What's deliberately NOT here yet:
//   * `init_gdt_and_interrupts`, `init_memory`, `breakpoint`,
//     `halt_forever` — they belong with step 4 (ExitBootServices +
//     kernel_run handoff), where the UEFI path joins the same arch-
//     neutral code the BIOS path drives. Until then the UEFI entry
//     stays in `entry_uefi.rs` and parks in a halt loop after
//     printing the banner — it never reaches the shared kernel body
//     that needs those functions.
//
// Post-ExitBootServices `_print` will need to switch off ConOut
// (firmware services are invalidated at that point) onto a real
// arch serial driver — COM1/16550 on x86_64, PL011 on aarch64.
// Tracked under step 4.

mod serial;

pub use serial::_print;

/// Initialise the architecture's console. Under UEFI the firmware has
/// already configured ConOut before transferring control to our entry,
/// so this is a no-op — kept as the named entry point so the shared
/// kernel body can call `arch::init_console()` target-agnostically.
pub fn init_console() {
    // Intentionally empty — see module docstring.
}
