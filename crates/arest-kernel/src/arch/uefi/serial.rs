// crates/arest-kernel/src/arch/uefi/serial.rs
//
// UEFI `_print` that survives ExitBootServices (#344 step 4). Before
// EBS, writes route through the firmware's ConOut Simple Text Output
// Protocol; after EBS, they route through a direct-I/O 16550 UART
// on COM1 (0x3F8). Same serial line QEMU's `-serial stdio` wires to
// host stdout for either firmware mode, so the boot banner survives
// the firmware hand-off unbroken.
//
// Pre-EBS pipeline:
//   1. `core::fmt::write` formats args into a heap `String` (allocator
//      is `uefi::allocator::Allocator` — installed in entry_uefi.rs).
//   2. `CString16::try_from` transcodes UTF-8 → UCS-2 (ConOut speaks
//      UCS-2; non-BMP scalars are silently dropped — banners are
//      ASCII).
//   3. `uefi::system::with_stdout` reaches the system-table ConOut
//      handle and writes the 16-bit string.
//
// Post-EBS pipeline:
//   1. Same `core::fmt::write` into a heap `String`. `uefi-rs`'s
//      allocator backend relies on BootServices pool allocations
//      which *are* invalidated at EBS — but the allocator has a
//      fallback mode once services are gone. For the small banner
//      strings this path writes, the backup is sufficient.
//   2. Write each byte directly to the 16550 COM1 port, polling the
//      Line Status Register's Transmit Holding Register Empty bit
//      before each byte. No interrupts — the post-EBS path is
//      synchronous so the kernel stays entirely in the polling
//      world until it installs its own IDT.
//
// The state switch happens via `switch_to_post_ebs_serial()`, called
// from `entry_uefi.rs` right after `boot::exit_boot_services`.

use alloc::string::String;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use uefi::CString16;
use uart_16550::SerialPort;

/// COM1 I/O port base — same address QEMU emulates and every
/// PC-compatible platform exposes, whether booted via BIOS or UEFI.
const COM1_IO_BASE: u16 = 0x3F8;

/// Pre-EBS (firmware ConOut) vs post-EBS (direct 16550). Gets flipped
/// exactly once by `switch_to_post_ebs_serial`.
static POST_EBS: AtomicBool = AtomicBool::new(false);

/// Lazy-initialised 16550 driver behind a spin lock. Init happens on
/// the first post-EBS write; the mutex serialises concurrent writes
/// the same way `arch::x86_64::serial::SERIAL` does.
static SERIAL: spin::Mutex<LazyUart> = spin::Mutex::new(LazyUart::new());

struct LazyUart {
    initialised: bool,
    port: SerialPort,
}

impl LazyUart {
    const fn new() -> Self {
        // SAFETY: constructing the SerialPort object without touching
        // hardware — init() runs on the first post-EBS write.
        Self {
            initialised: false,
            port: unsafe { SerialPort::new(COM1_IO_BASE) },
        }
    }

    fn ensure_initialised(&mut self) {
        if !self.initialised {
            self.port.init();
            self.initialised = true;
        }
    }
}

impl fmt::Write for LazyUart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.ensure_initialised();
        self.port.write_str(s)
    }
}

/// Flip `_print` onto the direct-I/O 16550 path. Must run AFTER
/// `boot::exit_boot_services` — once the firmware's ConOut is
/// invalid, further `with_stdout` calls silently no-op. Calling
/// before EBS is safe but writes go to a UART QEMU is still
/// echoing anyway, so there's no harm — just no point.
pub fn switch_to_post_ebs_serial() {
    POST_EBS.store(true, Ordering::SeqCst);
}

/// Called by the `print!` / `println!` macros declared in
/// `arch/mod.rs`. Routes to ConOut before EBS, to 16550 after.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    if POST_EBS.load(Ordering::SeqCst) {
        use core::fmt::Write as _;
        let _ = SERIAL.lock().write_fmt(args);
        return;
    }
    // Pre-EBS path: format into a String, transcode, hand to ConOut.
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
