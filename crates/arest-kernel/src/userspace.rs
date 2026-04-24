// crates/arest-kernel/src/userspace.rs
//
// Ring-3 descent and smoke-test payload.
//
// Sec-6.1 (this commit): map a user text page + user stack page via
// memory::map_user_page, copy a hand-assembled test payload in, flip
// the text page to read-only-executable, build an iretq frame on the
// kernel stack, and execute `iretq` to enter CPL=3 at the payload's
// entry point.
//
// Sec-6.2 (Task 9): the payload will invoke SYS_yield + SYS_exit and
// return control through the SYSCALL gate; until that gate is
// installed, the payload's first `syscall` instruction triggers a
// #GP (IA32_EFER.SCE = 0). That GP is caught by
// interrupts::general_protection_handler which routes CPL=3 faults
// to halt_on_exit(RING3_FAULT) → QEMU exit 35.
//
// Virtual layout:
//   USER_TEXT_BASE   0x0000_0040_0000_0000   (4 KiB, R+X, U=1)
//   USER_STACK_BASE  0x0000_0050_0000_0000   (4 KiB, R+W, U=1, NX=1)
//   USER_STACK_TOP   0x0000_0050_0000_1000   (grows down from here)
//
// Both addresses sit in the lower half (bit 47 = 0) so they pass the
// UserBuf canonical check that Task 10 adds.

use core::ptr;
use x86_64::VirtAddr;
use x86_64::instructions::port::Port;
use x86_64::structures::paging::PageTableFlags;

use crate::arch::gdt;
use crate::arch::memory;
use crate::println;

/// QEMU isa-debug-exit port.
const ISA_DEBUG_EXIT_PORT: u16 = 0xf4;

/// Virtual base of the user text page.
pub const USER_TEXT_BASE: u64  = 0x0000_0040_0000_0000;
/// Virtual base of the user stack page.
pub const USER_STACK_BASE: u64 = 0x0000_0050_0000_0000;
/// Initial ring-3 RSP — one page above USER_STACK_BASE.
pub const USER_STACK_TOP: u64  = USER_STACK_BASE + 0x1000;

/// Smoke-test exit codes written to the isa-debug-exit port.
pub mod exit_code {
    /// Smoke test reached SYS_exit cleanly.
    pub const SUCCESS:      u8 = 0x10;
    /// A CPU exception (#PF / #GP / #UD) was delivered from CPL=3.
    pub const RING3_FAULT:  u8 = 0x11;
    /// Kernel panic occurred during the smoke test.
    pub const KERNEL_PANIC: u8 = 0xFF;
}

/// Hand-assembled ring-3 test payload.
///
/// Drives the syscall gate end-to-end:
///
///   1. SYS_yield  — proves the trampoline + dispatch + sysretq path.
///   2. SYS_system — empty key + empty input, validates the ρ-app
///                   surface even with no-op args.
///   3. SYS_fetch  with a kernel-half pointer (0xFFFF_8000_…) — proves
///                   `UserBuf::from_raw` rejects the malicious pointer
///                   with `EFault` (-14) BEFORE any kernel-side read
///                   happens. The dispatcher's early-trace `println!`
///                   makes this rejection visible in the smoke log so
///                   the harness can assert the security boundary
///                   was exercised.
///   4. SYS_exit(SUCCESS) — writes 0x10 to the isa-debug-exit port,
///                   QEMU exits with code (0x10 << 1) | 1 = 33.
///
/// asm source (NASM-flavoured):
///   mov rax, 5                       ; SYS_yield
///   syscall
///   mov rax, 0                       ; SYS_system
///   xor rdi, rdi
///   xor rsi, rsi
///   xor rdx, rdx
///   xor r10, r10
///   xor r8,  r8
///   xor r9,  r9
///   syscall
///   ; Kernel-half pointer probe — must be rejected with EFault.
///   mov rax, 1                       ; SYS_fetch
///   mov rdi, 0xFFFF800000000000      ; kernel-half ptr (illegal)
///   mov rsi, 4                       ; len
///   xor rdx, rdx                     ; out_ptr (null OK — never reached)
///   xor r10, r10                     ; out_cap
///   syscall
///   ; (not checking RAX here; the early-trace println in dispatch_inner
///   ;  surfaces the syscall, and UserBuf::from_raw's EFault is the
///   ;  expected return — visible in the smoke harness assertions.)
///   mov rax, 6                       ; SYS_exit
///   mov rdi, 0x10                    ; exit_code::SUCCESS
///   syscall
///   ud2
pub const TEST_PAYLOAD: &[u8] = &[
    // mov rax, 5  (SYS_yield)
    0x48, 0xC7, 0xC0, 0x05, 0x00, 0x00, 0x00,
    // syscall
    0x0F, 0x05,
    // mov rax, 0  (SYS_system)
    0x48, 0xC7, 0xC0, 0x00, 0x00, 0x00, 0x00,
    // xor rdi, rdi
    0x48, 0x31, 0xFF,
    // xor rsi, rsi
    0x48, 0x31, 0xF6,
    // xor rdx, rdx
    0x48, 0x31, 0xD2,
    // xor r10, r10
    0x4D, 0x31, 0xD2,
    // xor r8, r8
    0x4D, 0x31, 0xC0,
    // xor r9, r9
    0x4D, 0x31, 0xC9,
    // syscall
    0x0F, 0x05,
    // mov rax, 1  (SYS_fetch)
    0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00,
    // movabs rdi, 0xFFFF800000000000  (REX.W + B8+rd id64) — kernel-half.
    // imm64 little-endian = 00 00 00 00 00 80 FF FF (LSB → MSB).
    0x48, 0xBF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0xFF, 0xFF,
    // mov rsi, 4
    0x48, 0xC7, 0xC6, 0x04, 0x00, 0x00, 0x00,
    // xor rdx, rdx
    0x48, 0x31, 0xD2,
    // xor r10, r10
    0x4D, 0x31, 0xD2,
    // syscall
    0x0F, 0x05,
    // mov rax, 6  (SYS_exit)
    0x48, 0xC7, 0xC0, 0x06, 0x00, 0x00, 0x00,
    // mov rdi, 0x10
    0x48, 0xC7, 0xC7, 0x10, 0x00, 0x00, 0x00,
    // syscall
    0x0F, 0x05,
    // ud2
    0x0F, 0x0B,
];

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

/// Map the user text + stack pages, copy the payload in, and descend
/// to ring 3 via `iretq`. Diverges.
pub fn launch_test_payload() -> ! {
    // 1. Memory subsystem must be live before we can mint user pages.
    //    The smoke branch in main.rs skips the normal init path, so
    //    set up memory::init here just for the pages map_user_page
    //    needs. (This is a localised workaround until Task 6 folds
    //    the smoke path into the normal init order.)
    //    NOTE: the `boot_info` is owned by main.rs; we can't re-init
    //    memory here without it. Instead, require that main.rs run
    //    memory::init itself under the smoke feature. See main.rs.

    // 2. Map user text as RW so we can copy the payload in.
    memory::map_user_page(
        VirtAddr::new(USER_TEXT_BASE),
        PageTableFlags::WRITABLE,
    )
    .expect("map user text");

    // 3. Copy the payload bytes in.
    // SAFETY: USER_TEXT_BASE is freshly mapped, writable, not
    // aliased by any other pointer.
    unsafe {
        ptr::copy_nonoverlapping(
            TEST_PAYLOAD.as_ptr(),
            USER_TEXT_BASE as *mut u8,
            TEST_PAYLOAD.len(),
        );
    }

    // 4. Flip user text to read-only-executable. NX stays clear so
    //    execution works; WRITABLE is removed so user code can't
    //    patch itself. The U bit stays set.
    memory::remap_user_page_flags(
        VirtAddr::new(USER_TEXT_BASE),
        PageTableFlags::empty(),
    )
    .expect("remap user text RX");

    // 5. Map user stack as RW + NX.
    memory::map_user_page(
        VirtAddr::new(USER_STACK_BASE),
        PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
    )
    .expect("map user stack");

    println!("  userspace: mapped user text + stack, descending to ring 3");

    // 6. Build an iretq frame and descend.
    descend_to_user(USER_TEXT_BASE, USER_STACK_TOP);
}

/// Build the iretq frame on the current stack and execute `iretq`.
/// CPU pops SS, RSP, RFLAGS, CS, RIP in that order and transitions
/// to CPL=3 at `entry_rip` with RSP=`user_rsp`.
#[inline(never)]
fn descend_to_user(entry_rip: u64, user_rsp: u64) -> ! {
    let sel = gdt::selectors();
    // user_code_segment / user_data_segment in x86_64 0.15 already
    // set RPL=3 on their returned SegmentSelector, so these values
    // are directly usable in the iretq frame.
    let user_cs = sel.user_cs64.0 as u64;
    let user_ss = sel.user_ss.0   as u64;
    let rflags  = 0x202u64; // IF=1 + reserved bit 1

    // SAFETY: User text + stack pages have been mapped with U=1 and
    // the payload copied in. The iretq pops exactly five quadwords
    // off the current kernel stack, which we push ourselves in the
    // order the CPU expects. Execution diverges into ring 3 and
    // never returns.
    unsafe {
        core::arch::asm!(
            "push {user_ss}",
            "push {user_rsp}",
            "push {rflags}",
            "push {user_cs}",
            "push {entry_rip}",
            "iretq",
            user_ss   = in(reg) user_ss,
            user_rsp  = in(reg) user_rsp,
            rflags    = in(reg) rflags,
            user_cs   = in(reg) user_cs,
            entry_rip = in(reg) entry_rip,
            options(noreturn),
        );
    }
}
