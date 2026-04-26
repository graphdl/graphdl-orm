// crates/arest-kernel/src/syscall/exit.rs
//
// Linux x86_64 syscalls 60 (`exit`) + 231 (`exit_group`). Both
// transition the calling Process to `Exited` and never return to
// userspace. Tier-1 conflates them — there's no thread model yet so
// the per-thread vs. per-process distinction is moot. The split lands
// once #560's POSIX threads epic introduces a Thread sibling for
// Process; until then `exit_group` is the canonical "process is done"
// surface (it's what musl's `_exit(3)` issues by default).
//
// State machine transition
// ------------------------
// Per #212's "state machine as derivation" pattern, the Process state
// IS the SM cell — a `Process_has_State` fact in the cell store
// (`process::process::Process::record_into_cells`). The handler sets
// `Process::state = ProcessState::Exited` via the
// `process::current_process_mut` accessor; the next time `system::
// apply` runs, the cell will reflect the new state.
//
// Why diverge (return `!`)
// ------------------------
// `exit` and `exit_group` MUST NOT return to userspace — there's no
// userspace stack frame to return to (the syscall's caller has just
// asked to be torn down). The return type `!` makes this an
// invariant the type system enforces: any code path leading to
// `Ok(_)` would fail to type-check. The dispatcher can still embed
// the call inside an `i64`-returning match arm because `!` coerces
// to any type.
//
// What happens after the SM transition
// ------------------------------------
// Tier-1 has no scheduler — the kernel runs a single Process at a
// time, started by the (future) #552 ring-3 gate. So once the
// Process is marked Exited, control flows back to the kernel's idle
// loop via `arch::halt_forever`. A real scheduler (#530) will
// instead pick the next runnable Process and trampoline into it.
//
// Exit status
// -----------
// The 32-bit exit status (rdi & 0xff per Linux's `wait(2)` masking
// convention — only the low byte is observable to the parent) is
// stashed on the Process for a future `waitpid`-like surface (#531).
// For tier-1 it's purely informational: the kernel idle loop doesn't
// branch on it.
//
// Test surface
// ------------
// The diverging signature makes `handle` itself untestable directly —
// you can't call it from a test without hanging the test runner. The
// testable surface is `mark_exited`, which performs the SM transition
// + status stash but doesn't enter the halt loop. The dispatcher's
// `handle` call site is exercised end-to-end via the dispatcher's own
// integration tests (which verify the SM cell goes to Exited).

use crate::process::current_process_mut;
use crate::process::ProcessState;

/// Handle an `exit(status)` or `exit_group(status)` syscall. Marks the
/// calling Process's state cell as `Exited`, stashes the exit status,
/// and never returns. Tier-1 idles the kernel via `arch::halt_forever`;
/// once #530's scheduler lands, this will instead `yield` to the next
/// runnable Process.
///
/// The `status` argument is the rdi register cast to `i32` per the
/// Linux convention — exit status is signed but only the low 8 bits
/// are observable to the parent (`wait(2)` masks with `0xff`). We keep
/// the full int so a future signed-status check has the bits.
///
/// SAFETY: this function diverges. Returning would be a kernel bug —
/// the caller has already torn down its userspace frame; there's no
/// rsp to restore. The `!` return type is the type system's enforcement.
pub fn handle(status: i32) -> ! {
    mark_exited(status);
    // Tier-1: no scheduler. Idle the kernel forever — the user's
    // demo workload (the static "hello world" Linux ELF) has done
    // its job; there's nothing else to run. A real scheduler (#530)
    // will instead `yield()` to the next runnable process; if the
    // exit-side process was the last, the scheduler will idle by
    // halt-then-poll-IRQ rather than the unconditional pause-loop
    // here.
    crate::arch::halt_forever()
}

/// Side-effect of `handle` that's testable — performs the state
/// machine transition + status stash without entering the halt loop.
/// Same source of truth as `handle`'s side-effect; the split lets
/// the unit tests assert on the post-condition without hanging.
///
/// Idempotent: calling twice on the same process leaves the state at
/// `Exited` and clobbers the status with the second call. (Linux's
/// `exit_group` is also idempotent in this sense — once a process is
/// torn down it can't be un-torn-down; double-exit just no-ops.)
///
/// If `current_process_mut` reports no current process (tier-1 boots
/// with no process registered until the eventual #552 ring-3 gate
/// installs one), the call is a no-op. Same shape as
/// `arch::uefi::time::now_ms` — the function is safe to call before
/// its prerequisite is initialised; callers that need the prerequisite
/// initialised guard at the call site.
pub fn mark_exited(status: i32) {
    current_process_mut(|maybe_proc| {
        if let Some(proc) = maybe_proc {
            proc.state = ProcessState::Exited;
            proc.exit_status = Some(status);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::current_process_install;
    use crate::process::current_process_uninstall;
    use crate::process::Process;

    /// `mark_exited(0)` transitions the registered current process to
    /// `Exited` and stashes the status. The test installs a fresh
    /// Process, calls `mark_exited`, and reads back the state via the
    /// same accessor.
    #[test]
    fn mark_exited_transitions_state_to_exited() {
        // Fresh address space + Process.
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        // Install as current process so the handler can find it.
        // Drop guard runs `uninstall` on test exit so cross-test
        // pollution can't happen (the static is process-wide).
        current_process_install(proc);
        mark_exited(0);
        current_process_mut(|maybe_proc| {
            let proc = maybe_proc.expect("current process must be installed");
            assert_eq!(proc.state, ProcessState::Exited);
            assert_eq!(proc.exit_status, Some(0));
        });
        current_process_uninstall();
    }

    /// `mark_exited` is a no-op when no current process is installed.
    /// Same shape as `arch::uefi::time::now_ms` returning 0 before
    /// `init_time` ran — the function tolerates pre-init calls.
    #[test]
    fn mark_exited_no_op_when_no_current_process() {
        // Make sure no process is installed (defensive, in case a
        // prior test forgot to uninstall).
        current_process_uninstall();
        // Should not panic; should not affect any state.
        mark_exited(42);
        current_process_mut(|maybe_proc| {
            assert!(maybe_proc.is_none());
        });
    }

    /// `mark_exited` clobbers status on repeated calls — the second
    /// status wins. Documents the idempotent-but-clobbering behaviour
    /// the docstring promises.
    #[test]
    fn mark_exited_repeated_calls_clobber_status() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(11, address_space);
        current_process_install(proc);
        mark_exited(1);
        mark_exited(2);
        current_process_mut(|maybe_proc| {
            let proc = maybe_proc.expect("current process must be installed");
            assert_eq!(proc.state, ProcessState::Exited);
            assert_eq!(proc.exit_status, Some(2));
        });
        current_process_uninstall();
    }

    /// The status argument is preserved verbatim — including negative
    /// values (a C program can `exit(-1)` — the kernel sees the i32
    /// faithfully even though the parent's `wait` only sees the low
    /// byte).
    #[test]
    fn mark_exited_preserves_signed_status() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(13, address_space);
        current_process_install(proc);
        mark_exited(-1);
        current_process_mut(|maybe_proc| {
            let proc = maybe_proc.expect("current process must be installed");
            assert_eq!(proc.exit_status, Some(-1));
        });
        current_process_uninstall();
    }
}
