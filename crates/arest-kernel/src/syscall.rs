// crates/arest-kernel/src/syscall.rs
//
// SYSCALL/SYSRETQ gate for the AREST kernel.
//
// Two halves: a tiny hand-written assembly trampoline
// (`syscall_entry`) that the CPU enters when ring-3 code executes
// `syscall`, and a Rust `dispatch` function that owns the actual
// syscall bodies. The trampoline's only jobs are stack switching
// (via swapgs + per-cpu slots) and argument marshalling (syscall
// convention -> SysV C ABI).
//
// Calling conventions:
//
//   Linux-style syscall convention (caller side, ring 3):
//     RAX = syscall number
//     RDI = arg0
//     RSI = arg1
//     RDX = arg2
//     R10 = arg3   (RCX is clobbered by the `syscall` instruction;
//                   the 4th arg goes in R10 instead)
//     R8  = arg4
//     R9  = arg5
//     return value in RAX, negative = SyscallErr as i64
//
//   SysV C ABI (dispatch() signature in Rust):
//     arg0 -> RDI, arg1 -> RSI, arg2 -> RDX, arg3 -> RCX,
//     arg4 -> R8,  arg5 -> R9,  arg6 -> [rsp+0]
//     return value in RAX
//
// The trampoline shuffles syscall-convention registers into SysV C
// ABI positions before the `call dispatch` instruction, then reverses
// the shuffle on the way out.

#![allow(dead_code)] // Some constants stay for future syscalls (6.3/6.4).

use spin::Once;
use x86_64::VirtAddr;
use x86_64::registers::model_specific::{Efer, EferFlags, KernelGsBase, LStar, SFMask, Star};
use x86_64::registers::rflags::RFlags;

use crate::gdt;
use crate::userspace;

// ---------------------------------------------------------------------------
// Per-cpu save area accessed by the trampoline via gs:[offset]
// ---------------------------------------------------------------------------

/// Per-cpu save area the trampoline touches via `gs:[offset]`. The
/// GS base is seeded with the address of this struct in `init()`;
/// `swapgs` at entry makes these offsets reachable from the
/// trampoline without depending on any register other than GS.
///
/// Fixed layout — the trampoline asm references these by literal
/// byte offset. If the struct grows, update the offsets in the
/// `naked_asm!` block below.
#[repr(C)]
struct PerCpu {
    /// Kernel stack top. Loaded into RSP on entry.
    /// Byte offset 0.
    kernel_rsp: u64,
    /// Slot used to save the user RSP across the syscall.
    /// Byte offset 8.
    user_rsp_save: u64,
}

/// Single-tenant for now. Becomes per-cpu / per-tenant in 6.3.
static PER_CPU: Once<PerCpu> = Once::new();

// ---------------------------------------------------------------------------
// MSR init
// ---------------------------------------------------------------------------

/// Initialise SYSCALL/SYSRETQ. Must run after `gdt::init()` because
/// it consumes the kernel/user selectors the GDT produced.
pub fn init() {
    // Seed the per-cpu save area first so GS base points at real
    // memory before the first `syscall`.
    PER_CPU.call_once(|| PerCpu {
        kernel_rsp:   gdt::kernel_stack_top().as_u64(),
        user_rsp_save: 0,
    });
    let per_cpu_addr = PER_CPU.get().unwrap() as *const PerCpu as u64;

    // IA32_KERNEL_GS_BASE is swapped into GS on every `swapgs`.
    // Put the per-cpu area here; the CPU-visible GS base stays
    // zero (user's GS) until swapgs flips them.
    KernelGsBase::write(VirtAddr::new(per_cpu_addr));

    // IA32_STAR — holds syscall & sysret selector bases.
    // Star::write takes the four selectors explicitly and packs
    // them internally. Arguments:
    //   cs_sysret, ss_sysret, cs_syscall, ss_syscall
    //
    // Our GDT layout is:
    //   idx 1 KernelCS, idx 2 KernelSS, idx 3 UserCS32,
    //   idx 4 UserSS,   idx 5 UserCS64
    // STAR.SYSRET_CS_SS is idx 3 so the CPU adds +8 (UserSS) and
    // +16 (UserCS64) when executing sysretq in 64-bit mode. We
    // pass user_cs64 / user_ss / kernel_cs / kernel_ss and the
    // crate figures out the packing.
    let sel = gdt::selectors();
    Star::write(
        sel.user_cs64,   // cs_sysret — packs so that SYSRET + 16 loads UserCS64
        sel.user_ss,     // ss_sysret — packs so that SYSRET + 8 loads UserSS
        sel.kernel_cs,   // cs_syscall
        sel.kernel_ss,   // ss_syscall
    )
    .expect("invalid STAR selectors");

    // IA32_LSTAR — RIP to load on `syscall`. Cast via pointer first
    // per nightly's function_casts_as_integer lint.
    LStar::write(VirtAddr::new(syscall_entry as *const () as usize as u64));

    // IA32_FMASK — bits cleared from RFLAGS on `syscall`. We mask
    // IF so interrupts are disabled during the trampoline, DF so
    // the direction flag is known, TF so single-step doesn't fire
    // in the trampoline.
    SFMask::write(
        RFlags::INTERRUPT_FLAG
            | RFlags::DIRECTION_FLAG
            | RFlags::TRAP_FLAG,
    );

    // Enable SCE (syscall extensions) in EFER. Without this bit
    // the `syscall` instruction raises #UD.
    unsafe {
        Efer::update(|flags| {
            flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
        });
    }
}

// ---------------------------------------------------------------------------
// Trampoline
// ---------------------------------------------------------------------------

/// The SYSCALL entry point. CPU jumps here from ring 3.
///
/// # Invariants on entry
/// - CPL=0, CS=kernel_cs, SS=kernel_ss (CPU-set via STAR)
/// - RCX = user RIP, R11 = user RFLAGS (CPU-saved)
/// - RSP still points at the user stack
/// - IF=0 (masked via SFMASK)
/// - GS base still points at the user's TLS
#[unsafe(naked)]
pub unsafe extern "C" fn syscall_entry() {
    // SAFETY: naked function — we own prologue/epilogue entirely.
    core::arch::naked_asm!(
        // 1. Switch GS base to the per-cpu kernel area.
        "swapgs",
        // 2. Save user RSP, load kernel RSP.
        "mov gs:[8], rsp",       // PerCpu.user_rsp_save
        "mov rsp, gs:[0]",       // PerCpu.kernel_rsp
        // 3. Save user RFLAGS and RIP (CPU put them in r11 / rcx).
        "push r11",
        "push rcx",
        // 4. Save callee-saved regs (SysV requires we preserve them
        //    across the call; dispatch may clobber caller-saved).
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        // 5. Shuffle syscall-convention args -> SysV C ABI.
        //    syscall: nr=RAX, a0=RDI, a1=RSI, a2=RDX, a3=R10, a4=R8, a5=R9
        //    SysV   : nr=RDI, a0=RSI, a1=RDX, a2=RCX, a3=R8,  a4=R9, a5=[rsp]
        "push r9",               // SysV 7th arg (a5)
        "mov r9,  r8",           // SysV arg5 = syscall a4
        "mov r8,  r10",          // SysV arg4 = syscall a3
        "mov rcx, rdx",          // SysV arg3 = syscall a2
        "mov rdx, rsi",          // SysV arg2 = syscall a1
        "mov rsi, rdi",          // SysV arg1 = syscall a0
        "mov rdi, rax",          // SysV arg0 = syscall nr
        // 6. Call dispatch — returns i64 in rax.
        "call {dispatch}",
        // 7. Drop the stacked a5 arg.
        "add rsp, 8",
        // 8. Restore callee-saved.
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        // 9. Restore user RIP / RFLAGS into rcx / r11.
        "pop rcx",
        "pop r11",
        // 10. Restore user RSP, swap GS, return to user.
        "mov rsp, gs:[8]",
        "swapgs",
        "sysretq",
        dispatch = sym dispatch,
    );
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

// Syscall numbers.
pub const SYS_SYSTEM:   u64 = 0;
pub const SYS_FETCH:    u64 = 1;
pub const SYS_STORE:    u64 = 2;
pub const SYS_SNAPSHOT: u64 = 3;
pub const SYS_ROLLBACK: u64 = 4;
pub const SYS_YIELD:    u64 = 5;
pub const SYS_EXIT:     u64 = 6;

/// Negative errno-style return codes. Subset of Linux values for
/// familiarity; only these four are used by 6.2.
#[repr(i64)]
#[derive(Clone, Copy)]
pub enum SyscallErr {
    EInval = -22,
    EFault = -14,
    ENoSys = -38,
    ENoMem = -12,
}

impl From<SyscallErr> for i64 {
    fn from(e: SyscallErr) -> i64 { e as i64 }
}

/// Rust entry point for the SYSCALL trampoline. SysV C ABI —
/// 7 args, 6 in regs + 1 on the stack.
///
/// Returns the user-side RAX value: non-negative on success, negative
/// `SyscallErr` on error.
#[no_mangle]
pub extern "C" fn dispatch(
    nr: u64,
    a0: u64,
    _a1: u64,
    _a2: u64,
    _a3: u64,
    _a4: u64,
    _a5: u64,
) -> i64 {
    match nr {
        SYS_YIELD => 0,
        SYS_EXIT  => userspace::halt_on_exit(a0 as u8),
        // Remaining numbers: ABI placeholder — Task 10 adds arg
        // validation. For now they all ENoSys.
        SYS_SYSTEM | SYS_FETCH | SYS_STORE | SYS_SNAPSHOT | SYS_ROLLBACK =>
            SyscallErr::ENoSys.into(),
        _ => SyscallErr::ENoSys.into(),
    }
}
