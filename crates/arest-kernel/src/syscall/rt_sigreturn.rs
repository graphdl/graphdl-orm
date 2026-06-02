// crates/arest-kernel/src/syscall/rt_sigreturn.rs
//
// Linux x86_64 syscall 15: `rt_sigreturn(void)`. Per #548 (the
// signal-handling plumbing). Returns from a signal handler: restores
// the register context + signal mask the kernel saved when it delivered
// the signal, resuming the interrupted instruction stream.
//
// Linux x86_64 number: `__NR_rt_sigreturn = 15`
// (`vendor/musl/arch/x86_64/bits/syscall.h.in:16`).
//
// How rt_sigreturn works on real Linux
// ------------------------------------
// `rt_sigreturn` is NOT called like an ordinary syscall by application
// code — it is the return address the kernel pushes onto the user stack
// (via the `sa_restorer` field of `k_sigaction`, which libc points at
// `__restore_rt`, `vendor/musl/src/signal/sigaction.c`) when it
// delivers a signal to a handler. When the handler executes its
// terminal `ret`, control lands in the restorer stub, which issues
// `syscall` with rax = 15. The kernel then:
//   1. reads the `rt_sigframe` the delivery path pushed onto the user
//      stack (the `ucontext` with the saved `mcontext` = the
//      interrupted GP registers, plus the pre-signal `uc_sigmask`),
//   2. restores the saved signal mask into the thread,
//   3. restores the saved registers (so the interrupted code resumes
//      exactly where the signal hit), and
//   4. does NOT return through the normal syscall-return path — the
//      restored RIP/RSP/etc. take over.
//
// The delivery-track dependency (honest scope for #548)
// -----------------------------------------------------
// Steps 1, 3, 4 require a *signal-delivery* mechanism: the kernel must
// already have (a) interrupted a ring-3 thread, (b) snapshotted its
// registers, (c) built the `rt_sigframe` on the user stack, and (d)
// redirected RIP to the handler. None of that exists in tier-1 — the
// trampoline (`process::trampoline::invoke`) returns
// `NotYetImplemented` (no ring-3 execution yet; #526 GDT/TSS + #527
// page tables + #552 ring-3 gate are pending), and signal *delivery*
// itself is the #549/#550/#551 track that THIS task is the foundation
// for. There is, by construction, no live register frame to restore.
//
// So #548 ships the plumbing #549+ will complete:
//   * the per-process saved-context SLOT (`SignalState::saved_context`,
//     parked by `push_context`, taken by `pop_context`),
//   * the mask-restore half — `pop_context` restores the pre-delivery
//     `uc_sigmask` into the live thread mask (the part that is pure
//     bookkeeping and fully testable without a VM), and
//   * this handler, which drives `pop_context` and reports the outcome.
//
// The live general-register restore (step 3's RIP/RSP/GP-regs takeover)
// is gated on the delivery track the same way `arch_prctl`'s MSR write
// is gated behind `cfg(uefi, x86_64)` and `brk`/`mmap`'s real
// page-table install is deferred to the boot-integration track: the
// kernel-side STATE transition is implemented + tested here; the
// instruction-level context switch lands when there's a ring-3 context
// to switch into. When #549+ wires delivery, it will call
// `pop_context` to recover the `SavedContext`, then perform the
// register takeover from the user-stack `rt_sigframe`; this handler's
// contract (drive `pop_context`, restore the mask) does not change.
//
// Return value
// ------------
// On real Linux `rt_sigreturn` does not "return a value" in the C
// sense — the restored context's rax is whatever the interrupted code
// had. In tier-1 (no register takeover yet) the handler returns 0 on a
// successful pop (a handler was executing and its context was
// restored), and -EINVAL when called with no handler executing
// (`rt_sigreturn` outside signal context is a programming error /
// hostile call; Linux's behaviour there is to consume whatever garbage
// is on the stack, often killing the process — we take the defensive
// -EINVAL path rather than act on an absent frame).
//
// errno values
//   EINVAL = 22 — called with no signal handler executing (no saved
//                 context to restore)
//   ESRCH  = 3  — no current process installed (see rt_sigaction)

use crate::process::current_process_signals;
use crate::syscall::dispatch::EINVAL;
use crate::syscall::rt_sigaction::ESRCH;

/// Handle an `rt_sigreturn()` syscall. Drives `SignalState::pop_context`
/// to restore the saved signal mask (and, on the future delivery track,
/// the saved register frame). Returns 0 when a handler was executing
/// (context restored), -EINVAL when none was, -ESRCH when no process is
/// installed.
///
/// Takes no meaningful arguments — Linux's `rt_sigreturn` reads the
/// `rt_sigframe` from the user stack (rsp), not from argument registers.
/// The dispatcher passes the register slots through but they are unused
/// here (the saved-context slot is the tier-1 stand-in for the
/// user-stack frame until the delivery track wires the real stack read).
pub fn handle() -> i64 {
    current_process_signals(|maybe_sig| {
        let sig = match maybe_sig {
            Some(s) => s,
            None => return -ESRCH,
        };
        // Pop the saved context: restores the pre-delivery signal mask
        // and clears the handler-active slot. `None` ⇒ no handler was
        // running ⇒ rt_sigreturn called out of context.
        match sig.pop_context() {
            Some(_ctx) => {
                // A handler was executing; its context (mask) is now
                // restored. The live register takeover from the
                // user-stack rt_sigframe lands with the #549+ delivery
                // track — see module docstring. Report success.
                0
            }
            None => -EINVAL,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::signal::SavedContext;
    use crate::process::{current_process_install, current_process_uninstall, Process};

    fn with_process<F: FnOnce()>(body: F) {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        current_process_install(Process::new(7, address_space));
        body();
        current_process_uninstall();
    }

    /// With a saved context parked (simulating mid-handler), the handler
    /// restores the saved mask, clears the handler-active slot, and
    /// returns 0.
    #[test]
    fn restores_saved_context_and_returns_zero() {
        with_process(|| {
            current_process_signals(|s| {
                let s = s.unwrap();
                // Simulate delivery: the pre-signal mask had bit 4 set;
                // delivery then widened the live mask to "everything".
                s.replace_mask(u64::MAX);
                s.push_context(SavedContext {
                    saved_mask: 1 << 4,
                    rip: 0x40_1234,
                    rsp: 0x7fff_8000,
                });
                assert!(s.in_handler());
            });
            // rt_sigreturn restores.
            let r = handle();
            assert_eq!(r, 0);
            current_process_signals(|s| {
                let s = s.unwrap();
                assert!(!s.in_handler(), "handler slot cleared");
                // Pre-delivery mask (bit 4) restored — NOT the widened
                // u64::MAX delivery mask.
                assert_eq!(s.blocked_mask(), 1 << 4);
            });
        });
    }

    /// Called with no handler executing, `rt_sigreturn` returns -EINVAL
    /// and leaves the (empty) saved-context slot alone.
    #[test]
    fn no_handler_returns_einval() {
        with_process(|| {
            current_process_signals(|s| assert!(!s.unwrap().in_handler()));
            assert_eq!(handle(), -EINVAL);
        });
    }

    /// With no current process installed, returns -ESRCH.
    #[test]
    fn no_process_returns_esrch() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();
        assert_eq!(handle(), -ESRCH);
    }

    /// Dispatch wiring: `dispatch(SYS_RT_SIGRETURN, ...)` routes here.
    /// Parks a context first so the pop succeeds and returns 0.
    #[test]
    fn dispatch_routes_rt_sigreturn() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        use crate::syscall::dispatch::{dispatch, SYS_RT_SIGRETURN};
        let address_space = AddressSpace::new(0x40_1000);
        current_process_install(Process::new(7, address_space));
        current_process_signals(|s| {
            s.unwrap().push_context(SavedContext {
                saved_mask: 0,
                rip: 0,
                rsp: 0,
            });
        });
        let r = dispatch(SYS_RT_SIGRETURN, 0, 0, 0, 0, 0, 0);
        assert_eq!(r, 0);
        current_process_uninstall();
    }
}
