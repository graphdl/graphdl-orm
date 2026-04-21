// crates/arest-kernel/src/userspace.rs
//
// Ring-3 descent and smoke-test payload. For Sec-6.1 we map a user
// text page + stack page, copy a hand-assembled test payload in,
// build an iretq frame, and transition CPL 0 -> 3. For Sec-6.2 the
// payload exercises the syscall gate and exits via SYS_exit.
//
// This file starts (Task 2) as a stub that only exercises the
// QEMU isa-debug-exit port so we can prove the harness wiring end-
// to-end before wiring up the privilege transition.

use x86_64::instructions::port::Port;

/// QEMU isa-debug-exit port. Writing a u32 here exits QEMU with
/// code (val << 1) | 1. Only meaningful when QEMU is launched with
/// `-device isa-debug-exit,iobase=0xf4,iosize=0x04`.
const ISA_DEBUG_EXIT_PORT: u16 = 0xf4;

/// Smoke-test exit codes written to the isa-debug-exit port.
pub mod exit_code {
    /// Smoke test reached SYS_exit cleanly.
    pub const SUCCESS: u8 = 0x10;
    /// A CPU exception (#PF / #GP / #UD) was delivered from CPL=3.
    pub const RING3_FAULT: u8 = 0x11;
    /// Kernel panic occurred during the smoke test.
    pub const KERNEL_PANIC: u8 = 0xFF;
}

/// Write `code` to QEMU's isa-debug-exit port then halt the CPU.
/// If the image is running on real hardware (no isa-debug-exit
/// device), the OUT instruction is a no-op and the function falls
/// through to the hlt loop.
pub fn halt_on_exit(code: u8) -> ! {
    // SAFETY: Port 0xf4 is only wired to isa-debug-exit; writing is
    // harmless on any other configuration (the IO space is unused).
    unsafe {
        let mut port = Port::<u32>::new(ISA_DEBUG_EXIT_PORT);
        port.write(code as u32);
    }
    loop {
        x86_64::instructions::hlt();
    }
}

/// Entry point for the smoke test. Task 2 stub — just writes the
/// success code and halts so we can prove the QEMU harness wiring
/// end-to-end before wiring up ring-3. Task 6 replaces this with
/// the real descent.
pub fn launch_test_payload() -> ! {
    halt_on_exit(exit_code::SUCCESS);
}
