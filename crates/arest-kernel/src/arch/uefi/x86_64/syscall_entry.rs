// crates/arest-kernel/src/arch/uefi/x86_64/syscall_entry.rs
//
// SYSCALL entry stub — the asm shim IA32_LSTAR points at. Final leg
// of the #552 ring-3 gate (paired with `gdt.rs`, `tss.rs`, and
// `syscall_msr.rs`).
//
// What happens when ring-3 code executes `syscall`
// ------------------------------------------------
// Per Intel SDM Vol 3 5.8.8:
//   1. RCX ← RIP (return address — the instruction after `syscall`)
//   2. R11 ← RFLAGS
//   3. RFLAGS ← RFLAGS & ~IA32_FMASK (we configured FMASK to clear
//      IF, so interrupts are now disabled)
//   4. CS  ← IA32_STAR[47:32]            — KERNEL_CS (0x08)
//   5. SS  ← IA32_STAR[47:32] + 8        — KERNEL_DS (0x10)
//   6. RIP ← IA32_LSTAR                   — this stub's entry
//   7. CPL ← 0
//
// Note: SYSCALL does NOT switch RSP — the user's RSP comes through
// unchanged. We continue running on the user's stack (which is a
// freshly-allocated 4 KiB page from `process::stack`, comfortably
// larger than our handler's frame).
//
// What this stub does
// -------------------
//   1. Save the syscall-clobberable register set on the stack:
//      first the user's RIP (in rcx) and RFLAGS (in r11), then
//      the callee-saved set (r12-r15, rbx, rbp).
//   2. Capture the user's RSP (which is `entry_RSP` — the value
//      the user had in RSP when `syscall` executed) into a
//      callee-saved slot.
//   3. Marshal the syscall args from Linux's SYSCALL ABI registers
//      (rax = number, rdi/rsi/rdx/r10/r8/r9 = args 1-6) into the
//      SysV-ABI argument registers our dispatcher expects
//      (rdi/rsi/rdx/rcx/r8/r9 + stack for arg 7). Linux uses r10
//      for arg 4 because rcx is clobbered by `syscall`.
//   4. Call `crate::syscall::dispatch::dispatch`. The return value
//      lands in rax — exactly where the Linux ABI wants the
//      syscall result.
//   5. Restore callee-saved + user RFLAGS + user RIP.
//   6. Build an IRETQ frame on the user's stack and execute IRETQ
//      to return to ring 3.
//
// Why IRETQ instead of SYSRETQ
// ----------------------------
// SYSRETQ in 64-bit mode loads CS from STAR[63:48]+16 and SS from
// STAR[63:48]+8. Our task-spec GDT layout (USER_CS=0x1B at index 3,
// USER_SS=0x23 at index 4) doesn't satisfy this — for SYSRETQ to
// produce USER_CS=0x1B it'd need STAR[63:48]=0x0B, but that gives
// SS=0x13 (= GDT slot 2 = kernel-DS) with RPL=3, a DPL mismatch.
// IRETQ takes the CS / SS values explicitly from its stack frame
// so we can drop in USER_CS / USER_SS literally.
//
// Why we don't switch to a kernel stack
// -------------------------------------
// A "real" SYSCALL handler swaps to a per-CPU kernel stack via
// SWAPGS + GS-relative loads (Linux's `entry_SYSCALL_64`). We don't
// have per-CPU storage yet (no scheduler — #530), and the user's
// stack is one page of dedicated, 16-aligned, kernel-allocated
// memory, so running our short handler on it is safe enough for
// tier-1. Once a multi-process scheduler lands, this stub will
// grow a SWAPGS + RSP swap.
//
// Naked function constraints
// --------------------------
// `naked_asm!` requires:
//   * The asm contains the ENTIRE function body (no Rust prologue
//     or epilogue is emitted).
//   * Every input must be a `const`, `sym`, or option — no `in(reg)`,
//     no `out(reg)` (those need register live-range tracking which
//     a naked function can't provide).
//   * The function must end in a control-transfer (ret / iretq /
//     jmp / etc.).

use core::arch::naked_asm;

/// SYSCALL entry stub. IA32_LSTAR points here. NEVER call this
/// from Rust code — it's reachable only via the `syscall`
/// instruction from ring 3.
///
/// The function is `extern "C"` for stable mangling; the actual
/// "calling convention" is the architectural SYSCALL behaviour
/// (registers as listed in the module docstring), not the SysV-C
/// ABI's register layout.
///
/// SAFETY: This is a CPU control-flow target. Calling it directly
/// from Rust would skip every prologue invariant the asm depends
/// on (rcx = user RIP, r11 = user RFLAGS, etc.) and corrupt the
/// CPU state. Only the CPU's `syscall` instruction may transfer
/// control here.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_entry() {
    naked_asm!(
        // -------- Phase 1: save user state --------
        // Save user RIP (rcx) and RFLAGS (r11) FIRST — both are
        // clobbered by `syscall` itself but we need them to
        // construct the IRETQ frame on return.
        "push rcx",                  // saved user RIP
        "push r11",                  // saved user RFLAGS
        // Save the rest of the callee-saved + caller-saved regs the
        // user expects to find unchanged. Per the syscall ABI,
        // every register EXCEPT rax/rcx/r11 is preserved. We've
        // dealt with rcx and r11; rax holds the syscall number
        // (which becomes our return value slot, so we don't need
        // to preserve it for the user). The args (rdi/rsi/rdx/r10/
        // r8/r9) are by-definition clobberable — they're the
        // syscall arguments. So we save the remaining callee-saved
        // (r12-r15, rbx, rbp). We also need to preserve the user's
        // RSP for the IRETQ frame at the end — we don't push it
        // explicitly because after all pops it's where RSP itself
        // points (entry RSP), which we recover via `lea rcx, [rsp +
        // 40]` after sub-ing the IRETQ frame slot.
        "push r15",
        "push r14",
        "push r13",
        "push r12",
        "push rbp",
        "push rbx",

        // -------- Phase 2: marshal args for dispatch --------
        // Linux x86_64 SYSCALL ABI:
        //   rax = number, rdi/rsi/rdx/r10/r8/r9 = args 1..6
        // Our dispatcher signature (`dispatch::dispatch`):
        //   fn dispatch(rax, rdi, rsi, rdx, r10, r8, r9) -> i64
        // SysV-C ABI register order for 7 args:
        //   rdi, rsi, rdx, rcx, r8, r9, [rsp]
        //
        // Mapping (in safe order — write rdi LAST so it doesn't
        // shadow itself):
        "push r9",                   // dispatcher 7th arg (r9 input)
        "mov r9, r8",                // dispatcher arg 6 = r8
        "mov r8, r10",               // dispatcher arg 5 = r10
        "mov rcx, rdx",              // dispatcher arg 4 = rdx
        "mov rdx, rsi",              // dispatcher arg 3 = rsi
        "mov rsi, rdi",              // dispatcher arg 2 = rdi
        "mov rdi, rax",              // dispatcher arg 1 = rax (syscall #)

        // -------- Phase 3: dispatch --------
        // Stack at this point (relative to entry RSP, decreasing):
        //   entry RSP - 8   : saved user RIP   (rcx)
        //   entry RSP - 16  : saved user RFLAGS (r11)
        //   entry RSP - 24  : saved user r15
        //   entry RSP - 32  : saved user r14
        //   entry RSP - 40  : saved user r13
        //   entry RSP - 48  : saved user r12
        //   entry RSP - 56  : saved user rbp
        //   entry RSP - 64  : saved user rbx
        //   entry RSP - 72  : dispatcher 7th arg (r9 value)
        //   entry RSP - 72  : <-- current RSP
        //
        // 9 pushes × 8 = 72 bytes. The `call` will push 8 more
        // (return address) for a total of 80 bytes. 80 % 16 = 0, so
        // we're 16-byte aligned at the call site — SysV ABI happy.
        //
        // The dispatcher is a Rust fn `dispatch::dispatch`. Use
        // `sym` to resolve the mangled name at link time.
        "call {dispatch}",
        // After return: rax = i64 return value (the syscall result
        // userspace will see in rax).

        // -------- Phase 4: restore user state --------
        // Pop the 7th-arg slot we pushed for the dispatch call.
        // Discard via a scratch register (rcx — about to be
        // overwritten by the user RFLAGS pop anyway).
        "pop rcx",                   // discard pushed-r9 slot

        // Restore callee-saved regs in reverse-push order.
        "pop rbx",
        "pop rbp",
        "pop r12",
        "pop r13",
        "pop r14",
        "pop r15",                   // user r15 restored
        "pop r11",                   // user RFLAGS restored
        "pop rcx",                   // user RIP restored

        // -------- Phase 5: build IRETQ frame and return --------
        // IRETQ frame (high → low memory address):
        //   [rsp + 32] : ss      = USER_SS (0x23)
        //   [rsp + 24] : rsp     = entry RSP (= user RSP)
        //   [rsp + 16] : rflags  = saved user RFLAGS (= r11)
        //   [rsp + 8]  : cs      = USER_CS (0x1B)
        //   [rsp + 0]  : rip     = saved user RIP (= rcx)
        //
        // After all pops above, RSP equals the entry RSP (we pushed
        // 9 × 8 bytes and popped them all). We now build the IRETQ
        // frame BELOW the entry RSP — same memory the user owned
        // before the syscall, which is safe to clobber because the
        // CPU pops the frame off the stack and the user's stack
        // pointer post-IRETQ is what we put in the rsp slot.
        //
        // Note: the user's rsp slot in the IRETQ frame is the
        // value of RSP at entry (= current RSP), not the RSP at
        // the time of the IRETQ instruction. The CPU pops 5 × 8
        // bytes from the kernel stack pointer and then loads the
        // popped rsp slot into RSP — restoring the user's stack
        // pointer to its pre-syscall value.
        "sub rsp, 40",
        "mov qword ptr [rsp + 0],  rcx",                 // user RIP
        "mov qword ptr [rsp + 8],  {user_cs}",            // CS = USER_CS
        "mov qword ptr [rsp + 16], r11",                 // user RFLAGS
        // The user RSP we want to restore IS the value of RSP at
        // entry — which is RSP + 40 right now (we just sub'd 40).
        // Compute it into rcx (free now), then store.
        "lea rcx, [rsp + 40]",
        "mov qword ptr [rsp + 24], rcx",                 // user RSP
        "mov qword ptr [rsp + 32], {user_ss}",            // SS = USER_SS

        "iretq",

        dispatch = sym crate::syscall::dispatch::dispatch,
        user_cs = const super::gdt::USER_CS as i64,
        user_ss = const super::gdt::USER_SS as i64,
    )
}
