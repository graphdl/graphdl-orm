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
// (the symmetric "release N waiters" operation, #545) removes up to
// `val` waiters from that same per-uaddr wait queue, marks the
// current process runnable again if it was the one parked on `uaddr`,
// and returns the number of waiters actually woken. Every other
// FUTEX_* op returns `-ENOSYS`.
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
/// `linux/include/uapi/linux/futex.h:FUTEX_WAKE`. #545 ships the real
/// implementation (`wake` below) against
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

/// FUTEX_LOCK_PI — acquire a priority-inheritance mutex. Per
/// `linux/include/uapi/linux/futex.h:FUTEX_LOCK_PI`. The futex word
/// holds the owner TID in its low 30 bits; userspace CASes 0<->TID for
/// the uncontended path and enters the kernel (this op) only on
/// contention or robust recovery. #547.
pub const FUTEX_LOCK_PI: u32 = 6;

/// FUTEX_UNLOCK_PI — release a PI mutex, handing off to the next
/// waiter. Per `linux/include/uapi/linux/futex.h:FUTEX_UNLOCK_PI`. #547.
pub const FUTEX_UNLOCK_PI: u32 = 7;

/// FUTEX_TRYLOCK_PI — non-blocking PI acquire. Per
/// `linux/include/uapi/linux/futex.h:FUTEX_TRYLOCK_PI`. #547.
pub const FUTEX_TRYLOCK_PI: u32 = 8;

/// Bit 31 of the futex word — set by the kernel to tell the owner
/// "there are waiters; release via FUTEX_UNLOCK_PI rather than a bare
/// userspace store-zero". Per `...:FUTEX_WAITERS`.
pub const FUTEX_WAITERS: u32 = 0x8000_0000;

/// Bit 30 of the futex word — set when the lock owner died while
/// holding it (robust mutexes). The next LOCK_PI / TRYLOCK_PI hands
/// ownership to the caller and returns -EOWNERDEAD so userspace runs
/// recovery. Per `...:FUTEX_OWNER_DIED`.
pub const FUTEX_OWNER_DIED: u32 = 0x4000_0000;

/// Low-30-bit mask isolating the owner TID (the top two bits are
/// FUTEX_OWNER_DIED + FUTEX_WAITERS). Per `...:FUTEX_TID_MASK`.
pub const FUTEX_TID_MASK: u32 = 0x3FFF_FFFF;

/// Linux errno "Operation not permitted". FUTEX_UNLOCK_PI returns this
/// when the caller is not the lock owner. `<asm-generic/errno-base.h>
/// :EPERM`.
pub const EPERM: i64 = 1;

/// Linux errno "Resource deadlock avoided". FUTEX_LOCK_PI / TRYLOCK_PI
/// return this when the caller already owns the lock.
/// `<asm-generic/errno.h>:EDEADLK`.
pub const EDEADLK: i64 = 35;

/// Linux errno "Owner died". FUTEX_LOCK_PI / TRYLOCK_PI return this
/// when the previous owner died holding the lock — the caller becomes
/// the new owner but MUST run robust-mutex recovery.
/// `<asm-generic/errno.h>:EOWNERDEAD`.
pub const EOWNERDEAD: i64 = 130;

/// Parsed owner-state of a PI futex word — `parse_futex_word`'s return.
/// The owner TID lives in the low 30 bits (`FUTEX_TID_MASK`); the top
/// two bits are flags (OWNER_DIED, WAITERS). A pure *reading* of the
/// word the PI handlers branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FutexWordState {
    /// Owner bits are zero — the lock is free.
    Unlocked,
    /// Owned by this (live) TID.
    LockedBy(u32),
    /// The OWNER_DIED flag is set; carries the dead owner's TID. The
    /// next acquirer takes ownership and must run robust recovery.
    OwnerDied(u32),
}

/// Classify a PI futex word into its owner-state. Pure — the handlers
/// read the userspace word via `read_u32` and hand the value here.
pub fn parse_futex_word(word: u32) -> FutexWordState {
    let tid = word & FUTEX_TID_MASK;
    if word & FUTEX_OWNER_DIED != 0 {
        FutexWordState::OwnerDied(tid)
    } else if tid == 0 {
        FutexWordState::Unlocked
    } else {
        FutexWordState::LockedBy(tid)
    }
}

/// True when the futex word's FUTEX_WAITERS bit is set — the kernel
/// has parked at least one waiter, so the owner must release through
/// FUTEX_UNLOCK_PI (hand-off) rather than a bare userspace store-zero.
pub fn has_waiters(word: u32) -> bool {
    word & FUTEX_WAITERS != 0
}

/// The outcome of attempting to acquire a PI futex — shared by
/// FUTEX_LOCK_PI and FUTEX_TRYLOCK_PI (they differ only on how they
/// treat `Contended`: LOCK_PI blocks, TRYLOCK_PI reports -EAGAIN).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiAcquire {
    /// The lock was free; the caller now owns it. Carries the new word
    /// (the caller's TID).
    Acquired(u32),
    /// The previous owner died holding the lock; the caller takes
    /// ownership (new word = caller TID, OWNER_DIED cleared) but the
    /// handler returns -EOWNERDEAD so userspace runs recovery.
    AcquiredOwnerDied(u32),
    /// Owned by a live other task. Carries the word with FUTEX_WAITERS
    /// set (LOCK_PI writes it then blocks; TRYLOCK_PI ignores it and
    /// returns -EAGAIN without registering a waiter).
    Contended(u32),
    /// The caller already owns the lock — a deadlock (-EDEADLK).
    Deadlock,
}

/// Decide the acquire outcome for a PI futex `word` and `caller_tid`.
/// Pure — no memory writes, no queue effects; the handler enacts the
/// result. Priority inheritance (boosting the owner to the caller's
/// priority) is deferred to the scheduler (#530), which owns the
/// priority model; this decides the ownership transition only.
pub fn pi_acquire(word: u32, caller_tid: u32) -> PiAcquire {
    match parse_futex_word(word) {
        FutexWordState::Unlocked => PiAcquire::Acquired(caller_tid),
        FutexWordState::OwnerDied(_) => PiAcquire::AcquiredOwnerDied(caller_tid),
        FutexWordState::LockedBy(owner) if owner == caller_tid => PiAcquire::Deadlock,
        FutexWordState::LockedBy(_) => PiAcquire::Contended(word | FUTEX_WAITERS),
    }
}

/// The outcome of releasing a PI futex — `pi_unlock`'s return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiUnlock {
    /// The caller is not the lock owner — rejected (-EPERM).
    NotOwner,
    /// No waiters; the lock is cleared (new word = 0).
    Cleared,
    /// Handed off to the next waiter. Carries the new word: the next
    /// owner's TID, with FUTEX_WAITERS set iff more waiters remain.
    HandedOff(u32),
}

/// Decide the release outcome for a PI futex. `caller_tid` must match
/// the word's current owner (else `NotOwner`). `next_waiter` is the
/// waiter the kernel dequeued to hand off to (None => no waiters);
/// `more_waiters` whether any remain after it (=> keep FUTEX_WAITERS on
/// the new owner word). Pure — the handler does the dequeue + word
/// write. Highest-priority-waiter selection is the scheduler's (#530);
/// tier-1 hands off in FIFO order via the existing wait queue.
pub fn pi_unlock(
    word: u32,
    caller_tid: u32,
    next_waiter: Option<u32>,
    more_waiters: bool,
) -> PiUnlock {
    if word & FUTEX_TID_MASK != caller_tid {
        return PiUnlock::NotOwner;
    }
    match next_waiter {
        None => PiUnlock::Cleared,
        Some(next) => {
            let new_word = if more_waiters {
                next | FUTEX_WAITERS
            } else {
                next
            };
            PiUnlock::HandedOff(new_word)
        }
    }
}

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
/// actually park yet), or the count of waiters woken (>= 0) from
/// WAKE. Returns `-EAGAIN` / `-EFAULT` / `-EINVAL` / `-ENOSYS` per the
/// errno table above.
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
        FUTEX_WAKE => wake(uaddr, val),
        FUTEX_LOCK_PI => lock_pi(uaddr),
        FUTEX_UNLOCK_PI => unlock_pi(uaddr),
        FUTEX_TRYLOCK_PI => trylock_pi(uaddr),
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

/// FUTEX_WAKE body. Remove up to `n` waiters from the `uaddr` wait
/// queue, mark the current process runnable again if it was the one
/// parked on `uaddr`, and return the number of waiters actually woken.
///
/// `n` is the `val` argument of `futex(uaddr, FUTEX_WAKE, val, ...)`:
/// the maximum number of waiters to release. Linux's canonical
/// `pthread_mutex_unlock` passes `1` (wake a single waiter);
/// `pthread_cond_broadcast` passes `INT_MAX` (wake every waiter). A
/// `val` of 0 wakes nobody and returns 0 — matching Linux's
/// `futex(uaddr, FUTEX_WAKE, 0)` no-op.
///
/// Address validation mirrors `wait`: a null `uaddr` returns -EFAULT
/// and an unaligned `uaddr` returns -EINVAL. Linux validates the
/// address on the WAKE side too — the kernel hashes the futex key from
/// `uaddr`, which requires the same alignment + non-null invariants the
/// WAIT path enforces. (Unlike `wait`, WAKE does NOT dereference
/// `*uaddr` — it only keys the wait queue by the address — so there's
/// no value-compare / -EAGAIN path here.)
///
/// Return value: the count of pids drained from the queue, as a
/// non-negative `i64`. Linux's FUTEX_WAKE returns the number woken;
/// userspace `pthread_cond_signal` ignores it, but a libc that checks
/// (e.g. an assertion that "exactly one waiter was present") sees the
/// real count.
///
/// Tier-1 limitation: there is no scheduler (#530) to actually resume
/// the woken pids on another CPU — the observable surface is the
/// queue drain plus the current process's state transition back to
/// `Running`. When #530 lands, the drained pids get pushed onto the
/// scheduler's run queue here; the return value (the count) stays the
/// same.
pub fn wake(uaddr: u64, n: u32) -> i64 {
    // Null-pointer guard — Linux rejects a null futex address on the
    // WAKE side too (the futex-key derivation faults on a null deref).
    if uaddr == 0 {
        return -EFAULT;
    }
    // 4-byte-alignment guard — the futex word must be naturally
    // aligned; Linux applies the same mask on WAKE as on WAIT.
    if uaddr & 0b11 != 0 {
        return -EINVAL;
    }
    // Drain up to `n` waiters from the per-uaddr queue. `wake_n`
    // returns the released pids in FIFO order (Linux's fair wake
    // convention) and prunes the queue entry when it empties. The
    // count of released pids is what FUTEX_WAKE reports to userspace.
    let woken = with_futex_table(|table| table.wake_n(uaddr, n as usize));
    // If the kernel's currently-installed process is the one that was
    // parked on this `uaddr`, transition it back to `Running` — the
    // WAKE counterpart to WAIT's `BlockedFutex(uaddr)` transition.
    // Tier-1 hosts one process at a time (no scheduler — #530), so the
    // current process is the only one whose state can be made runnable
    // here; the future scheduler will walk `woken` and mark every
    // released task runnable on its run queue. We gate the transition
    // on the pid actually appearing in `woken` so a WAKE that drained
    // some *other* (placeholder / already-reaped) pid doesn't
    // spuriously un-block the live process.
    if !woken.is_empty() {
        current_process_mut(|maybe_proc| {
            if let Some(proc) = maybe_proc {
                if proc.state == ProcessState::BlockedFutex(uaddr) && woken.contains(&proc.pid) {
                    proc.state = ProcessState::Running;
                }
            }
        });
    }
    // Linux raw-syscall convention: a successful FUTEX_WAKE returns the
    // count woken as a non-negative integer. `woken.len()` is bounded
    // by the queue length, which is far below `i64::MAX`, so the cast
    // is lossless.
    woken.len() as i64
}

/// The calling process's pid — the futex "TID" in tier-1's
/// single-thread model. Returns 0 when no process is installed (the
/// same placeholder `wait` uses pre-spawn / in the test harness).
fn current_pid() -> u32 {
    current_process_mut(|maybe| maybe.map(|p| p.pid).unwrap_or(0))
}

/// FUTEX_LOCK_PI body (#547). Acquire the priority-inheritance mutex at
/// `uaddr`. The futex word holds the owner TID in its low 30 bits:
///
///   * free word        -> acquire (word = caller TID), return 0.
///   * dead owner       -> acquire + return -EOWNERDEAD (robust recovery).
///   * caller is owner  -> -EDEADLK.
///   * owned (live)     -> set FUTEX_WAITERS, enqueue the caller, mark it
///                         BlockedFutex, return 0.
///
/// Tier-1 limitation: like `wait`, this does NOT actually park the
/// caller or perform the priority-inheritance boost — both ride the
/// scheduler (#530), which owns the run queue + the priority model. The
/// observable surface is the word write + the queue / state transition.
pub fn lock_pi(uaddr: u64) -> i64 {
    if uaddr == 0 {
        return -EFAULT;
    }
    if uaddr & 0b11 != 0 {
        return -EINVAL;
    }
    let caller_tid = current_pid();
    let word = read_u32(uaddr);
    match pi_acquire(word, caller_tid) {
        PiAcquire::Acquired(nw) => {
            write_u32(uaddr, nw);
            0
        }
        PiAcquire::AcquiredOwnerDied(nw) => {
            write_u32(uaddr, nw);
            -EOWNERDEAD
        }
        PiAcquire::Deadlock => -EDEADLK,
        PiAcquire::Contended(nw) => {
            // Tell the owner there are waiters (its release must hand off).
            write_u32(uaddr, nw);
            current_process_mut(|maybe| {
                if let Some(proc) = maybe {
                    proc.state = ProcessState::BlockedFutex(uaddr);
                }
            });
            with_futex_table(|table| table.enqueue(uaddr, caller_tid));
            0
        }
    }
}

/// FUTEX_TRYLOCK_PI body (#547). Non-blocking PI acquire: identical to
/// `lock_pi` on a free / dead-owner / self-owned word, but a contended
/// futex returns -EAGAIN WITHOUT registering a waiter or touching the
/// word (trylock never blocks, so it never sets FUTEX_WAITERS).
pub fn trylock_pi(uaddr: u64) -> i64 {
    if uaddr == 0 {
        return -EFAULT;
    }
    if uaddr & 0b11 != 0 {
        return -EINVAL;
    }
    let caller_tid = current_pid();
    let word = read_u32(uaddr);
    match pi_acquire(word, caller_tid) {
        PiAcquire::Acquired(nw) => {
            write_u32(uaddr, nw);
            0
        }
        PiAcquire::AcquiredOwnerDied(nw) => {
            write_u32(uaddr, nw);
            -EOWNERDEAD
        }
        PiAcquire::Deadlock => -EDEADLK,
        PiAcquire::Contended(_) => -EAGAIN,
    }
}

/// FUTEX_UNLOCK_PI body (#547). Release the PI mutex at `uaddr`. The
/// caller must be the current owner (else -EPERM). If waiters are
/// parked, hand the lock to the next one (FIFO in tier-1; the scheduler
/// #530 picks the highest priority) and keep FUTEX_WAITERS set iff more
/// remain; otherwise clear the word to 0.
pub fn unlock_pi(uaddr: u64) -> i64 {
    if uaddr == 0 {
        return -EFAULT;
    }
    if uaddr & 0b11 != 0 {
        return -EINVAL;
    }
    let caller_tid = current_pid();
    let word = read_u32(uaddr);
    // Ownership check before disturbing the wait queue.
    if word & FUTEX_TID_MASK != caller_tid {
        return -EPERM;
    }
    // Dequeue the hand-off target (if any) + note whether more remain.
    let (next, more) = with_futex_table(|table| {
        let drained = table.wake_n(uaddr, 1);
        let next = drained.first().copied();
        let more = !table.peek_waiters(uaddr).is_empty();
        (next, more)
    });
    match pi_unlock(word, caller_tid, next, more) {
        // Pre-checked above; defensive.
        PiUnlock::NotOwner => -EPERM,
        PiUnlock::Cleared => {
            write_u32(uaddr, 0);
            0
        }
        PiUnlock::HandedOff(nw) => {
            write_u32(uaddr, nw);
            // If the handed-off waiter is the currently-installed process,
            // make it runnable (tier-1; #530 walks the run queue).
            if let Some(next_tid) = next {
                current_process_mut(|maybe| {
                    if let Some(proc) = maybe {
                        if proc.state == ProcessState::BlockedFutex(uaddr)
                            && proc.pid == next_tid
                        {
                            proc.state = ProcessState::Running;
                        }
                    }
                });
            }
            0
        }
    }
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

/// Write a 4-byte little-endian u32 to a userspace virtual address —
/// the PI-futex counterpart to `read_u32`. The PI ops (#547) own the
/// futex word (unlike basic FUTEX_WAIT/WAKE where userspace CASes it),
/// so the kernel writes the new owner TID / FUTEX_WAITERS bit here.
///
/// SAFETY: the caller (`lock_pi` / `unlock_pi` / `trylock_pi`) has
/// validated `addr != 0` and 4-byte alignment. Under tier-1 identity
/// mapping the userspace VA doubles as a kernel VA; `write_volatile`
/// keeps the store from being elided / reordered across the
/// userspace-visible boundary. Once #527 lands real page tables this
/// routes through `process::address_space` / #561 copy_to_user.
pub fn write_u32(addr: u64, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::futex_table::with_futex_table;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
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
                let probes = [0_u64, 0x1000, 0x2000, 0x4040, 0x9000, 0xdead];
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
                0x9000,
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
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
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
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
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
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
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

    /// FUTEX_WAKE with a null `uaddr` returns -EFAULT before touching
    /// the wait queue. Linux rejects a null futex address on the WAKE
    /// side too.
    #[test]
    fn wake_null_uaddr_returns_efault() {
        let result = handle(0, FUTEX_WAKE, 1, 0, 0, 0);
        assert_eq!(result, -EFAULT);
    }

    /// FUTEX_WAKE with an unaligned `uaddr` returns -EINVAL. Same
    /// natural-alignment requirement the WAIT path enforces.
    #[test]
    fn wake_unaligned_uaddr_returns_einval() {
        assert_eq!(handle(0x4001, FUTEX_WAKE, 1, 0, 0, 0), -EINVAL);
        assert_eq!(handle(0x4002, FUTEX_WAKE, 1, 0, 0, 0), -EINVAL);
        assert_eq!(handle(0x4003, FUTEX_WAKE, 1, 0, 0, 0), -EINVAL);
    }

    /// FUTEX_WAKE on an empty (never-populated) queue returns 0 — no
    /// waiters to release, no panic, no spurious entry creation.
    #[test]
    fn wake_empty_queue_returns_zero() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();

        let result = handle(0x4040, FUTEX_WAKE, 1, 0, 0, 0);
        assert_eq!(result, 0, "no waiters parked → 0 woken");
        // The probe didn't create an entry for this uaddr. (Assert on
        // the specific key, not the global live count — sibling tests
        // share the process-wide table and may hold their own keys.)
        let waiters: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(0x4040).to_vec());
        assert_eq!(waiters, alloc::vec![] as alloc::vec::Vec<u32>);

        drain_global_futex_table();
    }

    /// FUTEX_WAKE wakes exactly one waiter and returns 1, leaving the
    /// rest parked. The headline `pthread_mutex_unlock` shape (wake a
    /// single waiter).
    #[test]
    fn wake_one_of_many_returns_one_and_drains_one() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        // Park three waiters on the same uaddr via the table directly.
        with_futex_table(|t| {
            t.enqueue(0x4040, 7);
            t.enqueue(0x4040, 11);
            t.enqueue(0x4040, 13);
        });
        // No live process for this case — exercise the pure
        // queue-drain surface.
        current_process_uninstall();

        let result = handle(0x4040, FUTEX_WAKE, 1, 0, 0, 0);
        assert_eq!(result, 1, "wake count clamps to val=1");

        // FIFO: pid 7 (first enqueued) was the one released.
        let waiters: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(0x4040).to_vec());
        assert_eq!(waiters, alloc::vec![11, 13], "only the head waiter woke");

        drain_global_futex_table();
    }

    /// FUTEX_WAKE waking N of M waiters returns N (the count actually
    /// woken), not M and not the requested cap. Spec headline: wake N
    /// of M waiters returns N.
    #[test]
    fn wake_n_of_m_returns_n() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        // M = 5 waiters parked.
        with_futex_table(|t| {
            for pid in [1u32, 2, 3, 4, 5] {
                t.enqueue(0x4040, pid);
            }
        });
        current_process_uninstall();

        // Wake N = 3.
        let result = handle(0x4040, FUTEX_WAKE, 3, 0, 0, 0);
        assert_eq!(result, 3, "N of M woken returns N");

        // The two not-woken waiters (FIFO tail) remain parked.
        let waiters: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(0x4040).to_vec());
        assert_eq!(waiters, alloc::vec![4, 5]);

        drain_global_futex_table();
    }

    /// FUTEX_WAKE with `val` exceeding the queue length wakes (and
    /// returns) every parked waiter — the queue empties and the entry
    /// is pruned. The `pthread_cond_broadcast` shape (val = INT_MAX).
    #[test]
    fn wake_cap_exceeding_queue_wakes_all() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        with_futex_table(|t| {
            t.enqueue(0x4040, 7);
            t.enqueue(0x4040, 11);
        });
        current_process_uninstall();

        // val far larger than the 2 parked waiters.
        let result = handle(0x4040, FUTEX_WAKE, u32::MAX, 0, 0, 0);
        assert_eq!(result, 2, "returns the count actually woken, not val");

        // This uaddr's queue is empty + pruned. (Check the specific
        // key rather than the global live count — sibling tests share
        // the process-wide table.)
        let waiters: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(0x4040).to_vec());
        assert_eq!(waiters, alloc::vec![] as alloc::vec::Vec<u32>, "drained queue is empty");

        drain_global_futex_table();
    }

    /// FUTEX_WAKE with `val == 0` wakes nobody and returns 0, leaving
    /// the queue untouched. Matches Linux's `futex(uaddr, FUTEX_WAKE,
    /// 0)` no-op.
    #[test]
    fn wake_zero_count_is_noop_returns_zero() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        with_futex_table(|t| t.enqueue(0x4040, 7));
        current_process_uninstall();

        let result = handle(0x4040, FUTEX_WAKE, 0, 0, 0, 0);
        assert_eq!(result, 0, "val=0 wakes nobody");

        // Waiter still parked.
        let waiters: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(0x4040).to_vec());
        assert_eq!(waiters, alloc::vec![7]);

        drain_global_futex_table();
    }

    /// FUTEX_WAKE respects the uaddr key — a wake on one uaddr leaves
    /// waiters parked on a different uaddr untouched. Spec headline:
    /// wake respects the uaddr key.
    #[test]
    fn wake_respects_uaddr_key() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        // Two distinct uaddrs, each with a parked waiter.
        with_futex_table(|t| {
            t.enqueue(0x1000, 7);
            t.enqueue(0x2000, 11);
        });
        current_process_uninstall();

        // Wake everything on 0x1000.
        let result = handle(0x1000, FUTEX_WAKE, u32::MAX, 0, 0, 0);
        assert_eq!(result, 1, "only 0x1000's lone waiter woke");

        // 0x2000's waiter is still parked — the key isolated the wake.
        let waiters_other: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(0x2000).to_vec());
        assert_eq!(waiters_other, alloc::vec![11], "other uaddr untouched");
        // 0x1000 drained.
        let waiters_woken: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(0x1000).to_vec());
        assert_eq!(waiters_woken, alloc::vec![] as alloc::vec::Vec<u32>);

        drain_global_futex_table();
    }

    /// End-to-end WAIT → WAKE round-trip through the public `handle`
    /// surface: a process parks via FUTEX_WAIT (state → BlockedFutex),
    /// then FUTEX_WAKE drains it and transitions the process back to
    /// `Running`. Proves the WAKE counterpart un-blocks the parked
    /// process's state machine.
    #[test]
    fn wait_then_wake_round_trips_process_state() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let cell: u32 = 42;
        let cell_addr = &cell as *const u32 as u64;
        assert_eq!(cell_addr & 0b11, 0, "cell must be 4-byte aligned");

        drain_global_futex_table();
        install_test_process(7);

        // WAIT: *uaddr == val → park, state becomes BlockedFutex.
        let wait_result = handle(cell_addr, FUTEX_WAIT, 42, 0, 0, 0);
        assert_eq!(wait_result, 0);
        current_process_mut(|maybe_proc| {
            let proc = maybe_proc.expect("process installed");
            assert_eq!(proc.state, ProcessState::BlockedFutex(cell_addr));
        });

        // WAKE: drain the lone waiter (pid 7), state becomes Running.
        let wake_result = handle(cell_addr, FUTEX_WAKE, 1, 0, 0, 0);
        assert_eq!(wake_result, 1, "the parked pid was woken");
        current_process_mut(|maybe_proc| {
            let proc = maybe_proc.expect("process installed");
            assert_eq!(
                proc.state,
                ProcessState::Running,
                "woken process transitions back to Running"
            );
        });

        // Queue drained.
        let waiters: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(cell_addr).to_vec());
        assert_eq!(waiters, alloc::vec![] as alloc::vec::Vec<u32>);

        // Cleanup.
        drain_global_futex_table();
        current_process_uninstall();
    }

    /// FUTEX_WAKE on a uaddr DIFFERENT from the one the current process
    /// parked on does NOT transition the process out of BlockedFutex —
    /// the state transition is gated on the uaddr key + the pid
    /// appearing in the woken set.
    #[test]
    fn wake_other_uaddr_leaves_blocked_process_blocked() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let cell: u32 = 42;
        let cell_addr = &cell as *const u32 as u64;
        assert_eq!(cell_addr & 0b11, 0);

        drain_global_futex_table();
        install_test_process(7);

        // Park pid 7 on cell_addr.
        let wait_result = handle(cell_addr, FUTEX_WAIT, 42, 0, 0, 0);
        assert_eq!(wait_result, 0);

        // Pre-seed a waiter on a DIFFERENT uaddr so the wake there
        // drains something (returns > 0) but must not touch pid 7's
        // state.
        with_futex_table(|t| t.enqueue(0x9000, 99));
        let wake_result = handle(0x9000, FUTEX_WAKE, 1, 0, 0, 0);
        assert_eq!(wake_result, 1);

        // pid 7 is still BlockedFutex(cell_addr) — the wake on 0x9000
        // didn't un-block it.
        current_process_mut(|maybe_proc| {
            let proc = maybe_proc.expect("process installed");
            assert_eq!(proc.state, ProcessState::BlockedFutex(cell_addr));
        });

        drain_global_futex_table();
        current_process_uninstall();
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
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
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
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
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

    // -- #547: PI-futex word codec + decision logic ------------------

    /// PI op-codes + word bit constants match the Linux uapi values.
    #[test]
    fn pi_op_constants_match_linux_uapi() {
        assert_eq!(FUTEX_LOCK_PI, 6);
        assert_eq!(FUTEX_UNLOCK_PI, 7);
        assert_eq!(FUTEX_TRYLOCK_PI, 8);
        assert_eq!(FUTEX_WAITERS, 0x8000_0000);
        assert_eq!(FUTEX_OWNER_DIED, 0x4000_0000);
        assert_eq!(FUTEX_TID_MASK, 0x3FFF_FFFF);
    }

    /// `parse_futex_word` classifies the owner state from the word's
    /// low bits + the OWNER_DIED flag; the WAITERS bit is orthogonal.
    #[test]
    fn parse_futex_word_classifies_owner_state() {
        assert_eq!(parse_futex_word(0), FutexWordState::Unlocked);
        assert_eq!(parse_futex_word(42), FutexWordState::LockedBy(42));
        assert_eq!(
            parse_futex_word(42 | FUTEX_WAITERS),
            FutexWordState::LockedBy(42)
        );
        assert_eq!(
            parse_futex_word(42 | FUTEX_OWNER_DIED),
            FutexWordState::OwnerDied(42)
        );
    }

    /// `has_waiters` reads the FUTEX_WAITERS bit.
    #[test]
    fn has_waiters_reads_the_waiters_bit() {
        assert!(!has_waiters(42));
        assert!(has_waiters(42 | FUTEX_WAITERS));
    }

    /// `pi_acquire` on a free word acquires it for the caller.
    #[test]
    fn pi_acquire_on_free_word_acquires() {
        assert_eq!(pi_acquire(0, 7), PiAcquire::Acquired(7));
    }

    /// `pi_acquire` on a word owned by a live other task is contended —
    /// the returned word gains the FUTEX_WAITERS bit (LOCK_PI then
    /// blocks; TRYLOCK_PI reports busy without writing it).
    #[test]
    fn pi_acquire_on_other_owner_is_contended_with_waiters_bit() {
        assert_eq!(pi_acquire(9, 7), PiAcquire::Contended(9 | FUTEX_WAITERS));
    }

    /// `pi_acquire` on a dead-owner word makes the caller the new owner
    /// (OWNER_DIED cleared) but flags robust recovery (-EOWNERDEAD).
    #[test]
    fn pi_acquire_on_owner_died_acquires_with_eownerdead() {
        assert_eq!(
            pi_acquire(9 | FUTEX_OWNER_DIED, 7),
            PiAcquire::AcquiredOwnerDied(7)
        );
    }

    /// `pi_acquire` where the caller already owns the lock is a
    /// deadlock (-EDEADLK).
    #[test]
    fn pi_acquire_on_self_owned_is_deadlock() {
        assert_eq!(pi_acquire(7, 7), PiAcquire::Deadlock);
    }

    /// `pi_unlock` by a non-owner is rejected (-EPERM).
    #[test]
    fn pi_unlock_by_non_owner_is_not_owner() {
        assert_eq!(pi_unlock(9, 7, None, false), PiUnlock::NotOwner);
    }

    /// `pi_unlock` by the owner with no waiters clears the lock (word→0).
    #[test]
    fn pi_unlock_owner_no_waiters_clears() {
        assert_eq!(pi_unlock(7, 7, None, false), PiUnlock::Cleared);
    }

    /// `pi_unlock` by the owner hands the lock to the next waiter; the
    /// new owner word keeps FUTEX_WAITERS only when more remain.
    #[test]
    fn pi_unlock_owner_hands_off_to_next_waiter() {
        assert_eq!(
            pi_unlock(7 | FUTEX_WAITERS, 7, Some(11), false),
            PiUnlock::HandedOff(11)
        );
        assert_eq!(
            pi_unlock(7 | FUTEX_WAITERS, 7, Some(11), true),
            PiUnlock::HandedOff(11 | FUTEX_WAITERS)
        );
    }

    // -- #547: PI-futex syscall handlers -----------------------------

    /// lock_pi rejects a null / unaligned uaddr like the basic ops.
    #[test]
    fn lock_pi_validates_uaddr() {
        let _g = CURRENT_PROCESS_TEST_LOCK.lock();
        assert_eq!(lock_pi(0), -EFAULT);
        assert_eq!(lock_pi(0x1001), -EINVAL);
    }

    /// lock_pi on a free futex acquires it: returns 0, the word now
    /// holds the caller's pid.
    #[test]
    fn lock_pi_on_free_word_acquires() {
        let _g = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        install_test_process(7);
        let mut cell: u32 = 0;
        let uaddr = &mut cell as *mut u32 as u64;
        assert_eq!(lock_pi(uaddr), 0);
        assert_eq!(cell, 7);
        current_process_uninstall();
    }

    /// lock_pi on a futex owned by another live task is contended: the
    /// caller enqueues + goes BlockedFutex, the word gains the WAITERS
    /// bit, the call returns 0 (tier-1 "blocks"; #530 does the real park
    /// + priority boost).
    #[test]
    fn lock_pi_on_contended_word_enqueues_and_sets_waiters() {
        let _g = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        install_test_process(7);
        let mut cell: u32 = 9; // owned by tid 9
        let uaddr = &mut cell as *mut u32 as u64;
        assert_eq!(lock_pi(uaddr), 0);
        assert_eq!(cell, 9 | FUTEX_WAITERS);
        let waiters = with_futex_table(|t| t.peek_waiters(uaddr).to_vec());
        assert_eq!(waiters, alloc::vec![7]);
        current_process_mut(|p| {
            assert_eq!(p.unwrap().state, ProcessState::BlockedFutex(uaddr))
        });
        with_futex_table(|t| {
            t.wake_n(uaddr, usize::MAX);
        });
        current_process_uninstall();
    }

    /// lock_pi where the caller already owns the lock is a deadlock.
    #[test]
    fn lock_pi_self_owned_is_deadlock() {
        let _g = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        install_test_process(7);
        let mut cell: u32 = 7; // already owned by caller
        let uaddr = &mut cell as *mut u32 as u64;
        assert_eq!(lock_pi(uaddr), -EDEADLK);
        assert_eq!(cell, 7); // unchanged
        current_process_uninstall();
    }

    /// lock_pi on a dead-owner futex hands ownership to the caller and
    /// returns -EOWNERDEAD; the word becomes the caller's pid
    /// (OWNER_DIED cleared).
    #[test]
    fn lock_pi_owner_died_acquires_with_eownerdead() {
        let _g = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        install_test_process(7);
        let mut cell: u32 = 9 | FUTEX_OWNER_DIED;
        let uaddr = &mut cell as *mut u32 as u64;
        assert_eq!(lock_pi(uaddr), -EOWNERDEAD);
        assert_eq!(cell, 7);
        current_process_uninstall();
    }

    /// trylock_pi acquires a free futex (0, word=pid); on a contended
    /// one it returns -EAGAIN and does NOT write the word or enqueue.
    #[test]
    fn trylock_pi_acquires_or_reports_busy() {
        let _g = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        install_test_process(7);
        let mut free: u32 = 0;
        let free_addr = &mut free as *mut u32 as u64;
        assert_eq!(trylock_pi(free_addr), 0);
        assert_eq!(free, 7);

        let mut held: u32 = 9;
        let held_addr = &mut held as *mut u32 as u64;
        assert_eq!(trylock_pi(held_addr), -EAGAIN);
        assert_eq!(held, 9); // unchanged, no WAITERS bit
        assert!(with_futex_table(|t| t.peek_waiters(held_addr).is_empty()));
        current_process_uninstall();
    }

    /// unlock_pi by a non-owner is -EPERM; by the owner with no waiters
    /// clears the word to 0.
    #[test]
    fn unlock_pi_perm_and_clear() {
        let _g = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        install_test_process(7);
        let mut not_mine: u32 = 9;
        let nm_addr = &mut not_mine as *mut u32 as u64;
        assert_eq!(unlock_pi(nm_addr), -EPERM);
        assert_eq!(not_mine, 9);

        let mut mine: u32 = 7;
        let mine_addr = &mut mine as *mut u32 as u64;
        assert_eq!(unlock_pi(mine_addr), 0);
        assert_eq!(mine, 0);
        current_process_uninstall();
    }

    /// unlock_pi by the owner with a waiter hands the lock off: the word
    /// becomes the next waiter's tid and the call returns 0.
    #[test]
    fn unlock_pi_hands_off_to_waiter() {
        let _g = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        install_test_process(7);
        let mut cell: u32 = 7 | FUTEX_WAITERS;
        let uaddr = &mut cell as *mut u32 as u64;
        with_futex_table(|t| t.enqueue(uaddr, 11));
        assert_eq!(unlock_pi(uaddr), 0);
        assert_eq!(cell, 11); // handed to 11; no more waiters → WAITERS clear
        assert!(with_futex_table(|t| t.peek_waiters(uaddr).is_empty()));
        current_process_uninstall();
    }

    /// The dispatcher routes the PI op-codes to their handlers.
    #[test]
    fn handle_dispatches_pi_ops() {
        let _g = CURRENT_PROCESS_TEST_LOCK.lock();
        drain_global_futex_table();
        install_test_process(7);
        let mut cell: u32 = 0;
        let uaddr = &mut cell as *mut u32 as u64;
        assert_eq!(handle(uaddr, FUTEX_LOCK_PI, 0, 0, 0, 0), 0);
        assert_eq!(cell, 7);
        assert_eq!(handle(uaddr, FUTEX_UNLOCK_PI, 0, 0, 0, 0), 0);
        assert_eq!(cell, 0);
        assert_eq!(handle(uaddr, FUTEX_TRYLOCK_PI, 0, 0, 0, 0), 0);
        assert_eq!(cell, 7);
        current_process_uninstall();
    }
}
