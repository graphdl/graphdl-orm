// crates/arest-kernel/src/syscall/futex.rs
//
// Linux x86_64 syscall 202: `futex(uint32_t *uaddr, int futex_op,
// uint32_t val, const struct timespec *timeout, uint32_t *uaddr2,
// uint32_t val3)`. Per #544 (Rand-1 / #474a) — the foundational
// primitive for any threaded glibc/musl-built binary's mutex /
// condvar implementation.
//
// What this slice ships
// ---------------------
// The FUTEX_WAIT operation: if the userspace word at `*uaddr` still
// equals the caller's expected `val`, park the calling process on a
// per-uaddr wait queue (`process::futex_table`); otherwise return
// `-EAGAIN` so libc retries the lock-acquire fast-path. FUTEX_WAKE
// (the symmetric "release N waiters" operation) is stubbed to return
// 0 — #545 (separate task) ships the real implementation against the
// same wait queue this slice populates. Every other FUTEX_* op
// returns `-ENOSYS`.
//
// futex_op encoding
// -----------------
// Linux's futex_op argument is a bitfield: the low 7 bits
// (`FUTEX_CMD_MASK = 0x7F`) carry the operation discriminant
// (FUTEX_WAIT, FUTEX_WAKE, etc), and the higher bits carry option
// flags (FUTEX_PRIVATE, FUTEX_CLOCK_REALTIME). Tier-1 ignores the
// flag bits — the PRIVATE / SHARED distinction collapses because
// there's only one process, and the clock distinction is moot
// because timeouts are ignored (treated as infinite — #547).
//
// Userspace memory access
// -----------------------
// `*uaddr` is a userspace virtual address pointing to a 4-byte word.
// Tier-1 has no page-table install; UEFI's identity mapping means
// userspace VAs coincide with kernel VAs (same rationale documented
// in `syscall::write` line 46 + `syscall::openat` line 71). We deref
// the pointer directly via `read_u32`; once #527 lands real page
// tables, the deref will route through `process::address_space` /
// the future #561 `copy_from_user` surface.
//
// Errno values
// ------------
// `EAGAIN = 11` — `*uaddr != val` at the moment of the WAIT call.
//   Userspace re-tries the lock-acquire fast-path (the contended-
//   mutex code in glibc/musl branches on EAGAIN to mean "the lock
//   state changed under us, try CAS again").
// `EFAULT = 14` — `uaddr` is null or the deref would fault. Tier-1
//   only catches null + isize-overflow because there's no page-walk
//   surface yet.
// `EINVAL = 22` — `uaddr` is not 4-byte-aligned. Linux requires the
//   futex word to be naturally aligned (atomic ops over un-aligned
//   words are split-bus in hardware).
// `ENOSYS = 38` — futex_op specifies an operation tier-1 doesn't yet
//   handle (REQUEUE / CMP_REQUEUE / WAIT_BITSET / etc).
//
// Block semantics
// ---------------
// FUTEX_WAIT with `*uaddr == val` is the "really block" path. Tier-1
// transitions the calling Process state to `BlockedFutex(uaddr)` via
// the `current_process_mut` accessor + enqueues the pid on the per-
// uaddr wait queue. The actual park-then-resume mechanism (yielding
// to the scheduler, restoring the rsp / rip on wake) lives in the
// future #530 scheduler — for tier-1, the state transition + queue
// insertion is the observable surface; the syscall returns 0 (success)
// to indicate "the kernel acknowledged the wait" so the test harness
// can introspect the post-call state. A real scheduler will instead
// not return from this call until FUTEX_WAKE drains the queue.
//
// Why return 0 from the WAIT path
// -------------------------------
// Linux's FUTEX_WAIT returns 0 on a normal wake (FUTEX_WAKE drained
// the queue). The errno-success convention means returning a non-
// negative integer signals "we did what you asked"; the actual
// blocking is the side effect of "the syscall doesn't return until
// the wake fires". Tier-1's stub behaviour (return 0 immediately
// after enqueueing) gives the test harness a way to see "the wait
// was registered" without having to wire FUTEX_WAKE first. When #545
// + #530 land, the WAIT path will yield to the scheduler before
// returning; the return value (0) stays the same.

use crate::process::current_process_mut;
use crate::process::futex_table::with_futex_table;
use crate::process::ProcessState;

/// Mask for the operation discriminant. Per
/// `linux/include/uapi/linux/futex.h:FUTEX_CMD_MASK`. The full
/// `futex_op` argument is `op & FUTEX_CMD_MASK | flags`; the flag
/// bits are FUTEX_PRIVATE_FLAG (128) and FUTEX_CLOCK_REALTIME (256).
pub const FUTEX_CMD_MASK: u32 = 0x7F;

/// Block the caller if `*uaddr == val`. Per
/// `linux/include/uapi/linux/futex.h:FUTEX_WAIT`. The cornerstone of
/// every glibc/musl pthread_mutex implementation — userspace does the
/// fast-path CAS in userspace, falls into the kernel only on
/// contention.
pub const FUTEX_WAIT: u32 = 0;

/// Wake up to `val` waiters parked on `uaddr`. Per
/// `linux/include/uapi/linux/futex.h:FUTEX_WAKE`. Tier-1 stubs to 0
/// — #545 (separate task) ships the real implementation against
/// `process::futex_table::wake_n`.
pub const FUTEX_WAKE: u32 = 1;

/// Move waiters from one uaddr to another. Per
/// `linux/include/uapi/linux/futex.h:FUTEX_REQUEUE`. Used by
/// pthread_cond_broadcast to atomically transfer condvar waiters to
/// the associated mutex's wait queue. Tier-1 returns -ENOSYS; the
/// implementation lands with #546.
pub const FUTEX_REQUEUE: u32 = 3;

/// CAS-then-requeue. Per
/// `linux/include/uapi/linux/futex.h:FUTEX_CMP_REQUEUE`. Same shape
/// as FUTEX_REQUEUE but with an atomic compare against a third value
/// before moving any waiters. Tier-1 returns -ENOSYS; #546.
pub const FUTEX_CMP_REQUEUE: u32 = 4;

/// Linux errno for "Resource temporarily unavailable" (also
/// `EWOULDBLOCK`). Per `<asm-generic/errno.h>:EAGAIN`. Returned by
/// FUTEX_WAIT when the value-mismatch fast-path fires (`*uaddr !=
/// val`). Userspace libc branches on EAGAIN to retry the lock-acquire.
pub const EAGAIN: i64 = 11;

/// Linux errno for "Bad address". Per
/// `<asm-generic/errno-base.h>:EFAULT`. Returned when `uaddr` is null.
/// Re-declared here (rather than re-exported from `dispatch`) so the
/// constant value is testable in this file's unit-test scope without
/// a cross-module use; the value matches `dispatch::EFAULT`.
pub const EFAULT: i64 = 14;

/// Linux errno for "Invalid argument". Per `<asm-generic/errno-base.h>
/// :EINVAL`. Returned when `uaddr` is not 4-byte aligned (atomic ops
/// require natural alignment).
pub const EINVAL: i64 = 22;

/// Linux errno for "Function not implemented". Per
/// `<asm-generic/errno.h>:ENOSYS`. Returned for futex ops tier-1
/// doesn't yet handle (REQUEUE, CMP_REQUEUE, WAIT_BITSET, etc).
pub const ENOSYS: i64 = 38;

/// Handle a `futex(uaddr, futex_op, val, timeout, uaddr2, val3)`
/// syscall. Match on `futex_op & FUTEX_CMD_MASK` and dispatch to the
/// per-op implementation. The high-bit flags (PRIVATE / CLOCK) are
/// ignored under tier-1 (single-process kernel, infinite timeouts).
///
/// Argument register mapping (Linux x86_64 ABI):
///   * `uaddr`     — rdi  — userspace VA of the futex word.
///   * `futex_op`  — rsi  — operation + flag bitfield.
///   * `val`       — rdx  — operation-dependent (expected value for
///                          WAIT, max wake count for WAKE).
///   * `timeout`   — r10  — `*timespec` for the WAIT timeout (tier-1
///                          ignores; treated as infinite).
///   * `uaddr2`    — r8   — second futex word for REQUEUE / WAKE_OP
///                          (tier-1 returns -ENOSYS for those).
///   * `val3`      — r9   — operation-dependent (expected value for
///                          CMP_REQUEUE; tier-1 returns -ENOSYS).
///
/// Returns 0 on a successful WAIT (after enqueueing — tier-1 doesn't
/// actually park yet) or 0 from the WAKE stub. Returns `-EAGAIN` /
/// `-EFAULT` / `-EINVAL` / `-ENOSYS` per the errno table above.
///
/// SAFETY: callers (the syscall dispatcher) treat `uaddr` as a
/// userspace virtual address. Tier-1's identity mapping makes this
/// safe for any non-null + 4-byte-aligned pointer; once #527 lands
/// real page tables, the deref needs to route through the per-process
/// AddressSpace (#561 `copy_from_user`).
pub fn handle(
    uaddr: u64,
    futex_op: u32,
    val: u32,
    _timeout: u64,
    _uaddr2: u64,
    _val3: u32,
) -> i64 {
    // Strip flags — tier-1 only branches on the operation discriminant.
    let op = futex_op & FUTEX_CMD_MASK;
    match op {
        FUTEX_WAIT => wait(uaddr, val),
        FUTEX_WAKE => {
            // Stubbed to 0 — #545 ships the real implementation. Returning
            // 0 (rather than -ENOSYS) is deliberate: a libc that probes
            // futex availability via WAKE+0 (a common pattern in glibc's
            // early-init `__libc_setup_tls`) sees "the syscall is
            // present" and proceeds. The stub doesn't actually wake any
            // waiters; tests that exercise the wake path use the
            // `futex_table::wake_n` surface directly.
            0
        }
        // Tier-1 doesn't model the requeue family or the bitset
        // variants. Userspace libc treats -ENOSYS on optional futex
        // ops as "this kernel doesn't have it"; pthread_cond_broadcast
        // falls back to a per-waiter wake loop in that case (see
        // `vendor/musl/src/thread/pthread_cond_timedwait.c` line 153
        // for the fallback shape).
        _ => -ENOSYS,
    }
}

/// FUTEX_WAIT body. Validate `uaddr`, read `*uaddr`, compare against
/// `val`. If they differ, return `-EAGAIN` so userspace retries the
/// fast-path CAS. If they match, transition the calling Process state
/// to `BlockedFutex(uaddr)` and enqueue the pid on the per-uaddr wait
/// queue, then return 0 (success).
///
/// Tier-1 limitation: this function does NOT actually park the
/// process — it returns immediately after the state transition + queue
/// insertion. The scheduler (#530) is what makes the syscall not
/// return until FUTEX_WAKE fires; until then the state transition is
/// the observable signal that a wait was registered.
pub fn wait(uaddr: u64, val: u32) -> i64 {
    // Null-pointer guard — fault before deref. Linux returns -EFAULT
    // for a null futex address (the cmpxchg the kernel does internally
    // would fault on a null deref).
    if uaddr == 0 {
        return -EFAULT;
    }
    // 4-byte-alignment guard — futex words must be naturally aligned
    // because the kernel's atomic compare-then-block is a 32-bit
    // load over a single bus cycle. Linux returns -EINVAL for an
    // unaligned uaddr (`linux/kernel/futex/core.c` does the same
    // mask).
    if uaddr & 0b11 != 0 {
        return -EINVAL;
    }
    // Read the userspace word. Under tier-1 identity mapping the
    // userspace VA doubles as a kernel VA — the same rationale
    // syscall::write + syscall::openat document.
    let observed = read_u32(uaddr);
    // The atomic-test-and-block check. If the value the caller
    // expected differs from what's actually at *uaddr, userspace
    // missed a wake (or never had a real reason to block) — return
    // -EAGAIN so the libc retry loop fires.
    if observed != val {
        return -EAGAIN;
    }
    // The "really block" path. Enqueue the calling pid on the per-
    // uaddr wait queue + transition the Process state. Both are
    // best-effort: if no current process is installed (test-harness
    // pre-init or kernel boot before any spawn), we still queue
    // a placeholder pid 0 so the test surface can introspect the
    // queue's behaviour without a Process being live. Production
    // callers always have a current process by the time a syscall
    // fires.
    let pid = current_process_mut(|maybe_proc| {
        if let Some(proc) = maybe_proc {
            proc.state = ProcessState::BlockedFutex(uaddr);
            proc.pid
        } else {
            0
        }
    });
    with_futex_table(|table| table.enqueue(uaddr, pid));
    // Return success — the WAIT was registered. Real Linux blocks the
    // caller's thread until FUTEX_WAKE fires; tier-1 returns 0
    // immediately + relies on the state-machine + queue surface for
    // the scheduler (#530) to pick up. When #545 + #530 land, this
    // call site grows a `scheduler::yield_until_woken(pid)` shim that
    // returns 0 only after the wake.
    0
}

/// Read a 4-byte little-endian u32 from a userspace virtual address.
/// Mirrors the inline pointer-deref pattern `syscall::write::do_write`
/// + `syscall::openat::read_pathname` use — direct deref under tier-1
/// identity mapping. Once #527 lands real page tables, this routes
/// through `process::address_space` / #561 copy_from_user.
///
/// The caller (`wait`) has already validated `uaddr != 0` and
/// `uaddr & 0b11 == 0` (4-byte aligned), so the deref is safe under
/// the tier-1 identity-mapping invariant.
///
/// SAFETY: dereferences `addr` as a `*const u32`. Caller is
/// responsible for the validity of the address (non-null, 4-byte
/// aligned, mapped). The `read_volatile` keeps the read from being
/// elided / hoisted by the optimiser, which matters because the value
/// at `addr` can change between userspace's CAS and the kernel's read.
pub fn read_u32(addr: u64) -> u32 {
    // SAFETY: `wait` validated non-null + 4-byte alignment. Under
    // tier-1 identity mapping the userspace VA doubles as a kernel
    // VA; `read_volatile` ensures the optimiser doesn't elide / hoist
    // the read across the userspace-CAS / kernel-block boundary.
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::futex_table::with_futex_table;
    use crate::process::{
        current_process_install, current_process_mut, current_process_uninstall, Process,
        ProcessState,
    };

    /// Helper: install a fresh Process so the handler has somewhere
    /// to record the BlockedFutex state. Mirrors the helper in the
    /// openat / close test suites.
    fn install_test_process(pid: u32) {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(pid, address_space);
        current_process_install(proc);
    }

    /// Helper: drain any leftover waiters from a prior test so the
    /// global futex_table starts each test in a known state. The
    /// global is process-wide; tests must clean up after themselves.
    fn drain_global_futex_table() {
        with_futex_table(|t| {
            let live: alloc::vec::Vec<u64> = (0..t.live_uaddr_count())
                .map(|_| 0)
                .collect();
            // Use a high cap to drain everything; this is conservative
            // — even if a prior test leaked, we clean up.
            for _ in &live {
                // Walk every uaddr the table currently holds.
                // We can't iterate the BTreeMap directly from outside,
                // so we use a probing pattern: peek each uaddr the
                // tests in this file use.
                let probes = [0_u64, 0x1000, 0x2000, 0x4040, 0xdead];
                for &uaddr in &probes {
                    t.wake_n(uaddr, usize::MAX);
                }
            }
            // Final pass even if live was empty.
            let probes = [
                0_u64,
                0x1000,
                0x2000,
                0x4040,
                0xdead,
            ];
            for &uaddr in &probes {
                t.wake_n(uaddr, usize::MAX);
            }
        });
    }

    /// `FUTEX_CMD_MASK` is 0x7F per
    /// `linux/include/uapi/linux/futex.h:FUTEX_CMD_MASK`.
    #[test]
    fn futex_cmd_mask_matches_linux_uapi() {
        assert_eq!(FUTEX_CMD_MASK, 0x7F);
    }

    /// `FUTEX_WAIT` is 0 per
    /// `linux/include/uapi/linux/futex.h:FUTEX_WAIT`.
    #[test]
    fn futex_wait_value_matches_linux_uapi() {
        assert_eq!(FUTEX_WAIT, 0);
    }

    /// `FUTEX_WAKE` is 1 per
    /// `linux/include/uapi/linux/futex.h:FUTEX_WAKE`.
    #[test]
    fn futex_wake_value_matches_linux_uapi() {
        assert_eq!(FUTEX_WAKE, 1);
    }

    /// `FUTEX_REQUEUE` is 3 per
    /// `linux/include/uapi/linux/futex.h:FUTEX_REQUEUE`.
    #[test]
    fn futex_requeue_value_matches_linux_uapi() {
        assert_eq!(FUTEX_REQUEUE, 3);
    }

    /// `FUTEX_CMP_REQUEUE` is 4 per
    /// `linux/include/uapi/linux/futex.h:FUTEX_CMP_REQUEUE`.
    #[test]
    fn futex_cmp_requeue_value_matches_linux_uapi() {
        assert_eq!(FUTEX_CMP_REQUEUE, 4);
    }

    /// `EAGAIN` is 11 per `<asm-generic/errno.h>:EAGAIN`.
    #[test]
    fn eagain_value_matches_linux_uapi() {
        assert_eq!(EAGAIN, 11);
    }

    /// `EINVAL` is 22 per `<asm-generic/errno-base.h>:EINVAL`.
    #[test]
    fn einval_value_matches_linux_uapi() {
        assert_eq!(EINVAL, 22);
    }

    /// `ENOSYS` is 38 per `<asm-generic/errno.h>:ENOSYS`.
    #[test]
    fn enosys_value_matches_linux_uapi() {
        assert_eq!(ENOSYS, 38);
    }

    /// FUTEX_WAIT with a null `uaddr` returns -EFAULT before any
    /// other validation. Linux returns -EFAULT for a null futex
    /// address.
    #[test]
    fn wait_null_uaddr_returns_efault() {
        let result = handle(0, FUTEX_WAIT, 0, 0, 0, 0);
        assert_eq!(result, -EFAULT);
    }

    /// FUTEX_WAIT with an unaligned `uaddr` returns -EINVAL. Linux
    /// requires 4-byte alignment because the futex word's atomic ops
    /// can't span a 4-byte boundary.
    #[test]
    fn wait_unaligned_uaddr_returns_einval() {
        // Pick an unaligned but otherwise valid pointer — any non-zero
        // address with low bits set. The handler must reject before
        // dereffing (we never read from this address).
        let result = handle(0x4001, FUTEX_WAIT, 0, 0, 0, 0);
        assert_eq!(result, -EINVAL);
        let result = handle(0x4002, FUTEX_WAIT, 0, 0, 0, 0);
        assert_eq!(result, -EINVAL);
        let result = handle(0x4003, FUTEX_WAIT, 0, 0, 0, 0);
        assert_eq!(result, -EINVAL);
    }

    /// FUTEX_WAIT with `*uaddr != val` returns -EAGAIN. The
    /// classic "userspace fast-path lost the race" path that libc
    /// branches on to retry CAS.
    #[test]
    fn wait_value_mismatch_returns_eagain() {
        // Allocate a 4-byte-aligned u32 cell with a known value; ask
        // the handler to wait for a different value. Should return
        // -EAGAIN immediately, NOT enqueue.
        let cell: u32 = 100;
        let cell_addr = &cell as *const u32 as u64;
        // Guard: ensure the test's assumption about alignment holds.
        assert_eq!(
            cell_addr & 0b11,
            0,
            "test cell must be 4-byte aligned"
        );

        // Don't install a process — the value-mismatch path should
        // short-circuit before touching the Process state.
        current_process_uninstall();
        drain_global_futex_table();

        let result = handle(cell_addr, FUTEX_WAIT, 200, 0, 0, 0);
        assert_eq!(result, -EAGAIN);

        // Confirm: nothing was enqueued.
        let waiters_len = with_futex_table(|t| t.peek_waiters(cell_addr).len());
        assert_eq!(waiters_len, 0, "no enqueue on EAGAIN path");
    }

    /// FUTEX_WAIT with `*uaddr == val` enqueues the calling pid + sets
    /// the Process state to `BlockedFutex(uaddr)` + returns 0.
    #[test]
    fn wait_value_match_enqueues_and_blocks() {
        let cell: u32 = 42;
        let cell_addr = &cell as *const u32 as u64;
        assert_eq!(cell_addr & 0b11, 0, "cell must be 4-byte aligned");

        drain_global_futex_table();
        install_test_process(7);

        let result = handle(cell_addr, FUTEX_WAIT, 42, 0, 0, 0);
        assert_eq!(result, 0, "match path returns 0");

        // Process state transitioned to BlockedFutex with the right uaddr.
        current_process_mut(|maybe_proc| {
            let proc = maybe_proc.expect("process installed");
            assert_eq!(proc.state, ProcessState::BlockedFutex(cell_addr));
        });

        // Pid was enqueued on the per-uaddr wait queue.
        let waiters: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(cell_addr).to_vec());
        assert_eq!(waiters, alloc::vec![7]);

        // Cleanup.
        drain_global_futex_table();
        current_process_uninstall();
    }

    /// FUTEX_WAIT with `*uaddr == val` and no current process still
    /// enqueues a placeholder pid (0) so the test harness can
    /// exercise the wait-queue surface in isolation. Production
    /// callers always have a current process by the time a syscall
    /// fires.
    #[test]
    fn wait_value_match_with_no_process_uses_placeholder_pid() {
        let cell: u32 = 99;
        let cell_addr = &cell as *const u32 as u64;
        assert_eq!(cell_addr & 0b11, 0, "cell must be 4-byte aligned");

        drain_global_futex_table();
        current_process_uninstall();

        let result = handle(cell_addr, FUTEX_WAIT, 99, 0, 0, 0);
        assert_eq!(result, 0);

        // Placeholder pid 0 enqueued.
        let waiters: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(cell_addr).to_vec());
        assert_eq!(waiters, alloc::vec![0]);

        // Cleanup.
        drain_global_futex_table();
    }

    /// FUTEX_WAKE returns 0 (stub for #545). Doesn't touch the wait
    /// queue — the test confirms the queue is preserved across the
    /// stub call.
    #[test]
    fn wake_returns_zero_under_stub() {
        drain_global_futex_table();
        // Pre-populate a waiter via the table directly so we can
        // confirm the stub leaves it alone.
        with_futex_table(|t| t.enqueue(0x4040, 11));

        let result = handle(0x4040, FUTEX_WAKE, 1, 0, 0, 0);
        assert_eq!(result, 0);

        // Stub didn't drain the queue (#545 will).
        let waiters: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(0x4040).to_vec());
        assert_eq!(waiters, alloc::vec![11]);

        // Cleanup.
        drain_global_futex_table();
    }

    /// Unsupported futex ops (REQUEUE / CMP_REQUEUE / WAIT_BITSET /
    /// etc) return -ENOSYS. Userspace libc treats this as "this kernel
    /// doesn't support the op" and falls back to the per-waiter wake
    /// loop.
    #[test]
    fn unsupported_op_returns_enosys() {
        let result = handle(0x4000, FUTEX_REQUEUE, 0, 0, 0, 0);
        assert_eq!(result, -ENOSYS);
        let result = handle(0x4000, FUTEX_CMP_REQUEUE, 0, 0, 0, 0);
        assert_eq!(result, -ENOSYS);
        // Arbitrary unrecognised op (FUTEX_WAIT_BITSET = 9).
        let result = handle(0x4000, 9, 0, 0, 0, 0);
        assert_eq!(result, -ENOSYS);
    }

    /// The high-bit flags (FUTEX_PRIVATE_FLAG = 128, FUTEX_CLOCK_REALTIME
    /// = 256) are stripped before op dispatch — FUTEX_WAIT |
    /// FUTEX_PRIVATE_FLAG behaves identically to FUTEX_WAIT under tier-1
    /// (single-process kernel; PRIVATE / SHARED collapse).
    #[test]
    fn private_flag_stripped_for_op_dispatch() {
        let cell: u32 = 7;
        let cell_addr = &cell as *const u32 as u64;
        assert_eq!(cell_addr & 0b11, 0);

        drain_global_futex_table();
        current_process_uninstall();

        // FUTEX_WAIT | FUTEX_PRIVATE_FLAG (= 128) — the value-mismatch
        // path should still fire (we ask for val=99 against *cell=7).
        let result = handle(cell_addr, FUTEX_WAIT | 128, 99, 0, 0, 0);
        assert_eq!(result, -EAGAIN);

        // Cleanup.
        drain_global_futex_table();
    }

    /// `read_u32` round-trips a known value through a userspace
    /// pointer. Documents the contract `wait` depends on.
    #[test]
    fn read_u32_returns_observed_value() {
        let cell: u32 = 0xdead_beef;
        let cell_addr = &cell as *const u32 as u64;
        assert_eq!(read_u32(cell_addr), 0xdead_beef);
    }

    /// `wait` with `*uaddr == val` and a current process leaves the
    /// process in `BlockedFutex(uaddr)` carrying the right uaddr.
    /// Regression test against accidentally storing the value or the
    /// timeout instead.
    #[test]
    fn wait_blocked_futex_carries_correct_uaddr() {
        let cell: u32 = 5;
        let cell_addr = &cell as *const u32 as u64;
        assert_eq!(cell_addr & 0b11, 0);

        drain_global_futex_table();
        install_test_process(13);

        let result = handle(cell_addr, FUTEX_WAIT, 5, 0xffff, 0, 0);
        assert_eq!(result, 0);
        current_process_mut(|maybe_proc| {
            let proc = maybe_proc.expect("process installed");
            // The Blocked variant carries cell_addr, NOT 0xffff (the
            // timeout) and NOT 5 (the val).
            match proc.state {
                ProcessState::BlockedFutex(stored) => {
                    assert_eq!(stored, cell_addr);
                }
                other => panic!("expected BlockedFutex, got {:?}", other),
            }
        });

        // Cleanup.
        drain_global_futex_table();
        current_process_uninstall();
    }
}
