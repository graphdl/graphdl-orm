// crates/arest-kernel/src/arch/uefi/serial.rs
//
// ConOut-backed `_print` for the UEFI build (#344 step 3). The shared
// `println!` macro at `arch::println!` calls into `arch::_print` —
// this file supplies the UEFI implementation so kernel code that uses
// `println!` works under x86_64-unknown-uefi / aarch64-unknown-uefi
// without arch-specific call sites.
//
// Pipeline per call:
//   1. `core::fmt::write` formats the args into a heap `String`
//      (allocator is `uefi::allocator::Allocator` — installed by
//      `entry_uefi.rs`).
//   2. `uefi::CString16::try_from` transcodes UTF-8 → UCS-2 (UEFI
//      ConOut speaks UCS-2; non-BMP scalars produce an Err that we
//      drop silently — the kernel banner is ASCII).
//   3. `uefi::system::with_stdout` reaches the system table's
//      ConOut handle and writes the C-style 16-bit string.
//
// Stays valid only until `BootServices::exit_boot_services`. After
// that, the system table fields backing `with_stdout` become invalid
// and writes silently no-op. Step 4 of the pivot replaces this with
// a real arch serial driver (16550 on x86_64-uefi → COM1 in QEMU,
// PL011 on aarch64-uefi → virt-pl011 in QEMU) once the UEFI path
// reaches the same post-firmware state the BIOS path lives in.

use alloc::string::String;
use core::fmt;
use uefi::CString16;

/// Called by the `print!` / `println!` macros declared in
/// `arch/mod.rs`. Same hidden symbol the BIOS path's
/// `arch::x86_64::serial::_print` exposes — the `pub use` chain
/// in `arch/mod.rs` selects the active implementation per target.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    use core::fmt::Write as _;
    let mut buf = String::new();
    if buf.write_fmt(args).is_err() {
        return;
    }
    let Ok(s16) = CString16::try_from(buf.as_str()) else { return };
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(&s16);
    });
}
