// crates/arest-kernel/src/arch/x86_64/serial.rs
//
// 8250 / 16550 UART driver on COM1 (I/O port 0x3F8). Provides:
//   - a `SERIAL` singleton behind a spin::Mutex,
//   - `_print` / `_eprint` helpers the macros call into,
//   - `print!` / `println!` macros (exported crate-wide).
//
// Why COM1 @ 0x3F8: every PC-compatible platform QEMU emulates has a
// legacy 8250 UART exposed here. No firmware probing needed — we can
// talk to it from the very first instruction after the bootloader
// hands control over.
//
// The mutex is `spin::Mutex`, not `std::sync::Mutex`, because we have
// no OS to block against. Lock contention on bare metal is resolved
// by spinning. For a single-core kernel the mutex is mostly a
// compile-time guard against split borrows; on SMP it's a real lock.
//
// Lives under `arch/x86_64/` (#344 step 2). The crate-root macros
// route `print!` through `$crate::arch::_print` which is re-exported
// from this module's `_print` — so adding an aarch64 serial driver
// later is a matter of providing a matching `arch::_print` without
// touching the shared kernel body or the macros.

use core::fmt;
use spin::Mutex;
use uart_16550::SerialPort;

/// COM1 I/O base on every PC-compatible platform.
const COM1_IO_BASE: u16 = 0x3F8;

/// Lazy-initialised SerialPort behind a spin lock. The `Mutex::new`
/// constructor is `const fn`, so this works as a `static` without a
/// OnceCell — `SERIAL.lock()` just runs `init()` once on first use
/// via `uart_16550::SerialPort::init()`.
pub static SERIAL: Mutex<LazyUart> = Mutex::new(LazyUart::new());

pub struct LazyUart {
    initialised: bool,
    port: SerialPort,
}

impl LazyUart {
    const fn new() -> Self {
        // SAFETY: we construct the SerialPort object but do not touch
        // the hardware until `init()` runs on first use.
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

/// Called by the `print!` / `println!` macros (declared in
/// `arch/mod.rs` so the same macros work on both arches). Locks the
/// serial port and writes the formatted string; panics are impossible
/// because `fmt::Write` for LazyUart never fails.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    use core::fmt::Write;
    let _ = SERIAL.lock().write_fmt(args);
}
