// crates/arest-kernel/src/process/futex_table.rs
//
// Per-uaddr futex wait queue (#544 — Rand-1 / #474a). Stores the set
// of process ids currently parked on each `uaddr` (the userspace
// virtual address of the futex word). Populated by `FUTEX_WAIT` (this
// slice — `crate::syscall::futex`); drained by `FUTEX_WAKE` (#545,
// separate task) which calls `wake_n(uaddr, n)` to release up to `n`
// waiters.
//
// Why a global table
// ------------------
// Linux's futex(2) is a kernel-wide rendezvous: any process can WAIT
// on a uaddr and any other process can WAKE that same uaddr (subject
// to address-space sharing — for SHARED futexes the uaddr is keyed by
// the underlying physical page; for PRIVATE futexes the keying folds
// the calling process's mm pointer in). Tier-1 collapses both shapes
// into a single per-uaddr key — there's no SMP, only one process is
// ever live at a time (no scheduler — #530), and the address space is
// identity-mapped (no MMU rewrites) so `uaddr` IS the unique key.
// When the scheduler + multi-process surface lands, the key will grow
// to `(uaddr, mm_id)` for FUTEX_PRIVATE; the table shape stays the
// same, only the key type shifts.
//
// Why BTreeMap rather than a Vec / HashMap
// -----------------------------------------
// The waiter set is sparse — a typical pthread_mutex-heavy program
// holds maybe a few hundred unique uaddrs at any time (one per
// contended lock / condvar). `BTreeMap<u64, Vec<ProcessId>>` keeps
// the storage proportional to the live set, gives O(log n) lookup /
// insert / remove, and walks in uaddr-numerical order which is what
// a future "list every blocked task" debug surface wants. Same shape
// as `process::fd_table::FdTable` (BTreeMap<i32, FdEntry>). HashMap
// would also work but pulls in `hashbrown` + a runtime hasher —
// extra dependencies for no win at tier-1's table sizes.
//
// `Vec<ProcessId>` value rather than `BTreeSet<ProcessId>`
// ---------------------------------------------------------
// Linux futex semantics: the same task can WAIT on the same uaddr
// only once (a second WAIT before the first is woken returns -EAGAIN
// from the value-mismatch check, OR blocks if the value still
// matches). So in practice the waiter set is a small ordered list,
// not a true set with dedup semantics. `Vec` matches the FIFO order
// FUTEX_WAKE wants (Linux wakes waiters in arrival order); `BTreeSet`
// would walk in pid order which is not what userspace expects.
//
// Why ProcessId is u32
// --------------------
// `Process::pid` is a `u32` per `process::process::Process::pid`.
// Aliased here as `ProcessId` so the futex_table API reads as
// "what kind of thing this is" rather than "an arbitrary u32".
// Stays a type alias rather than a newtype because tier-1 has no
// other use for a wrapped pid; if a future ergonomic concern (the
// raw u32 being confused with a fd or a count) surfaces, the alias
// can become a transparent newtype without changing any call site.
//
// What this module does NOT do (intentionally — see #545+)
//   * The actual park-then-resume mechanism. This table is a
//     book-keeping surface — adding a (uaddr, pid) pair records
//     "this pid asked to wait", removing it records "the kernel
//     released the wait". The scheduler (#530) drives the actual
//     CPU yield + resume; the syscall handler (`syscall::futex`)
//     transitions the Process state to `BlockedFutex(uaddr)` so the
//     scheduler skips it on the next dispatch.
//   * Timeout handling. FUTEX_WAIT with a non-NULL `timeout` argument
//     still enqueues the waiter — the timeout-expired path lands with
//     #547 (PI futex + real timeouts). For tier-1, timeout is ignored
//     (treated as infinite).
//   * FUTEX_REQUEUE / FUTEX_CMP_REQUEUE (atomic move waiters from one
//     uaddr to another). #546 ships those once the basic WAIT/WAKE
//     pair is wired.
//   * Cross-address-space coalescing for FUTEX_SHARED (the uaddr
//     resolves to a physical page that may be mapped at different
//     VAs in different processes). Tier-1 collapses to per-VA keys;
//     SHARED handling lands with the multi-process / scheduler epic.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Type alias for the per-process id used as the waiter identity. See
/// `process::process::Process::pid` — this matches that type exactly
/// so the syscall handler can pass `current_process.pid` straight in
/// without a cast.
pub type ProcessId = u32;

/// Per-uaddr wait queue. Maps each futex word's userspace virtual
/// address to the FIFO list of pids currently parked on it.
///
/// Constructed via `FutexTable::new` (empty); mutated via `enqueue` /
/// `wake_n`. `peek_waiters` is a read-only debug accessor used by the
/// unit tests.
///
/// `Default` so the global singleton's `spin::Mutex<FutexTable>`
/// initialises trivially (`FutexTable::default()` ≡ empty).
#[derive(Debug, Default)]
pub struct FutexTable {
    /// uaddr → ordered list of waiters. The list is FIFO — `enqueue`
    /// appends, `wake_n` drains from the front. Empty queues are
    /// pruned by `wake_n` so a future "list every parked uaddr"
    /// debug surface only sees live waiters.
    queues: BTreeMap<u64, Vec<ProcessId>>,
}

impl FutexTable {
    /// Construct an empty futex table. The global singleton uses
    /// `Default` so this constructor is mostly for the unit tests +
    /// the future per-process futex-table-isolation refactor (if SMP
    /// makes the global lock too coarse).
    pub fn new() -> Self {
        Self {
            queues: BTreeMap::new(),
        }
    }

    /// Park `pid` on `uaddr` — append to the per-uaddr FIFO. Called
    /// by the FUTEX_WAIT handler after the value-comparison check
    /// passes (i.e. `*uaddr == val`, so the caller really did want to
    /// block).
    ///
    /// Idempotent for distinct calls — the same `(uaddr, pid)` pair
    /// can be enqueued multiple times because Linux's futex semantics
    /// don't dedup; a process can only have one outstanding WAIT at a
    /// time per pthread_mutex contract, but the kernel-side table
    /// doesn't enforce that (the Process state machine transition to
    /// `BlockedFutex` is what enforces "one wait per process at a
    /// time" — a process that's already Blocked can't issue another
    /// syscall).
    pub fn enqueue(&mut self, uaddr: u64, pid: ProcessId) {
        self.queues.entry(uaddr).or_insert_with(Vec::new).push(pid);
    }

    /// Wake up to `n` waiters parked on `uaddr`. Returns the pids
    /// that were released, in FIFO order (the order they enqueued).
    ///
    /// If fewer than `n` waiters are queued, returns all of them and
    /// the queue becomes empty (and the entry is pruned from the
    /// table — `peek_waiters(uaddr)` will then return an empty slice
    /// because the BTreeMap entry is gone, not just emptied).
    ///
    /// `n == 0` is a no-op that returns an empty Vec — matches Linux's
    /// `futex(uaddr, FUTEX_WAKE, 0)` which "wakes 0 waiters" (i.e.
    /// reports the queue length without disturbing it; Linux returns
    /// the count, tier-1's stub returns 0). The check against the cap
    /// of `i64::MAX` would make this matter; for tier-1 it's
    /// essentially "drain at most n".
    ///
    /// Used by the (separate-task) FUTEX_WAKE handler — #545.
    pub fn wake_n(&mut self, uaddr: u64, n: usize) -> Vec<ProcessId> {
        let mut woken = Vec::new();
        let prune = match self.queues.get_mut(&uaddr) {
            Some(queue) => {
                let take = core::cmp::min(n, queue.len());
                // Drain from the front to preserve FIFO order — the
                // first waiter to enqueue is the first to wake, matching
                // Linux's "fair" wake-order convention.
                let drained: Vec<ProcessId> = queue.drain(..take).collect();
                woken.extend(drained);
                queue.is_empty()
            }
            None => false,
        };
        // Prune the entry if its queue is now empty so the table
        // doesn't accumulate dead uaddrs across the lifetime of the
        // kernel. A future "iterate every parked uaddr" debug surface
        // expects only live entries.
        if prune {
            self.queues.remove(&uaddr);
        }
        woken
    }

    /// Read-only view of the current waiter list for `uaddr`. Returns
    /// an empty slice if no one is parked there. Used by the unit tests
    /// + a future debug surface that wants to print the full waiter
    /// state without taking ownership.
    pub fn peek_waiters(&self, uaddr: u64) -> &[ProcessId] {
        self.queues
            .get(&uaddr)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Number of distinct uaddrs with live waiters. Used by the unit
    /// tests to confirm `wake_n` prunes empty entries; a future
    /// `getrlimit`-style surface might also expose this.
    pub fn live_uaddr_count(&self) -> usize {
        self.queues.len()
    }

    /// True when no waiters are parked anywhere. Convenience for the
    /// unit tests + a future "is the kernel idle" surface.
    pub fn is_empty(&self) -> bool {
        self.queues.is_empty()
    }
}

// -- Global singleton (#473a futex surface) -----------------------------
//
// The futex syscall handler (`crate::syscall::futex`) reaches the
// table via `with_futex_table(|t| ...)` rather than threading a
// `&mut FutexTable` through. Same shape as the kernel's other global
// mutable singletons (`process::current_process_mut`,
// `arch::uefi::memory::with_page_table`) — `spin::Mutex<FutexTable>`
// gives single-CPU-no-contention access; the closure shape stops the
// `&mut FutexTable` from leaking past the lock.

/// Singleton holding the kernel-wide futex wait queues. Empty at boot.
/// Populated by FUTEX_WAIT (this slice's syscall handler), drained by
/// FUTEX_WAKE (#545).
///
/// `spin::Mutex` rather than `RefCell` so a future SMP path doesn't
/// have to retrofit the lock — same rationale as
/// `process::process::CURRENT_PROCESS`.
static FUTEX_TABLE: spin::Mutex<FutexTable> = spin::Mutex::new(FutexTable {
    queues: BTreeMap::new(),
});

/// Run a closure against the global futex table, returning the closure's
/// result. Holds the singleton's `spin::Mutex` for the duration of the
/// closure — don't park / await inside (no async in the kernel today;
/// this is a "don't grow one" reminder).
///
/// Same closure shape as `process::current_process_mut`,
/// `process::current_process_fd_table` — consistent with the rest of
/// the kernel's singleton accessors.
pub fn with_futex_table<F, R>(f: F) -> R
where
    F: FnOnce(&mut FutexTable) -> R,
{
    let mut guard = FUTEX_TABLE.lock();
    f(&mut *guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `FutexTable::new` produces an empty table — no live uaddrs,
    /// `peek_waiters` returns empty slice for any address.
    #[test]
    fn new_table_is_empty() {
        let t = FutexTable::new();
        assert!(t.is_empty());
        assert_eq!(t.live_uaddr_count(), 0);
        assert_eq!(t.peek_waiters(0xdead_beef), &[] as &[ProcessId]);
    }

    /// `enqueue` adds a single waiter — `peek_waiters` reflects it,
    /// `live_uaddr_count` ticks to 1.
    #[test]
    fn enqueue_single_waiter_records_pid() {
        let mut t = FutexTable::new();
        t.enqueue(0x1000, 7);
        assert_eq!(t.peek_waiters(0x1000), &[7]);
        assert_eq!(t.live_uaddr_count(), 1);
        assert!(!t.is_empty());
    }

    /// `enqueue` preserves FIFO order — the first pid to enqueue is the
    /// first in the slice (and the first to wake).
    #[test]
    fn enqueue_multiple_waiters_preserves_fifo_order() {
        let mut t = FutexTable::new();
        t.enqueue(0x1000, 7);
        t.enqueue(0x1000, 11);
        t.enqueue(0x1000, 13);
        assert_eq!(t.peek_waiters(0x1000), &[7, 11, 13]);
        assert_eq!(t.live_uaddr_count(), 1);
    }

    /// `enqueue` against distinct uaddrs creates distinct queues.
    #[test]
    fn enqueue_distinct_uaddrs_creates_distinct_queues() {
        let mut t = FutexTable::new();
        t.enqueue(0x1000, 7);
        t.enqueue(0x2000, 11);
        assert_eq!(t.peek_waiters(0x1000), &[7]);
        assert_eq!(t.peek_waiters(0x2000), &[11]);
        assert_eq!(t.live_uaddr_count(), 2);
    }

    /// Same `(uaddr, pid)` enqueued twice records two entries — the
    /// table doesn't dedup. Linux semantics: the same task can't
    /// double-WAIT (the second WAIT happens after the first is
    /// woken, so the kernel sees them sequentially), but the table
    /// itself is happy to hold duplicates if the higher-level state
    /// machine asks for it.
    #[test]
    fn enqueue_same_pair_twice_records_two_entries() {
        let mut t = FutexTable::new();
        t.enqueue(0x1000, 7);
        t.enqueue(0x1000, 7);
        assert_eq!(t.peek_waiters(0x1000), &[7, 7]);
    }

    /// `wake_n(uaddr, 1)` wakes exactly one waiter and leaves the rest.
    #[test]
    fn wake_one_drains_one_waiter() {
        let mut t = FutexTable::new();
        t.enqueue(0x1000, 7);
        t.enqueue(0x1000, 11);
        let woken = t.wake_n(0x1000, 1);
        assert_eq!(woken, alloc::vec![7]);
        assert_eq!(t.peek_waiters(0x1000), &[11]);
    }

    /// `wake_n(uaddr, n)` returns waiters in FIFO order — the first to
    /// enqueue is the first to wake.
    #[test]
    fn wake_n_returns_waiters_in_fifo_order() {
        let mut t = FutexTable::new();
        t.enqueue(0x1000, 7);
        t.enqueue(0x1000, 11);
        t.enqueue(0x1000, 13);
        let woken = t.wake_n(0x1000, 2);
        assert_eq!(woken, alloc::vec![7, 11]);
        assert_eq!(t.peek_waiters(0x1000), &[13]);
    }

    /// `wake_n(uaddr, n)` where `n` exceeds the queue length drains
    /// every waiter and prunes the entry — `live_uaddr_count` drops
    /// to 0, `peek_waiters` returns empty.
    #[test]
    fn wake_n_exceeding_queue_drains_and_prunes() {
        let mut t = FutexTable::new();
        t.enqueue(0x1000, 7);
        t.enqueue(0x1000, 11);
        let woken = t.wake_n(0x1000, 99);
        assert_eq!(woken, alloc::vec![7, 11]);
        assert_eq!(t.peek_waiters(0x1000), &[] as &[ProcessId]);
        assert_eq!(t.live_uaddr_count(), 0);
        assert!(t.is_empty());
    }

    /// `wake_n` against an unknown uaddr returns an empty Vec — no
    /// panic, no spurious entry creation.
    #[test]
    fn wake_n_unknown_uaddr_returns_empty() {
        let mut t = FutexTable::new();
        let woken = t.wake_n(0x1000, 5);
        assert_eq!(woken, alloc::vec![] as alloc::vec::Vec<ProcessId>);
        assert!(t.is_empty());
    }

    /// `wake_n(uaddr, 0)` is a no-op — returns empty Vec, doesn't
    /// disturb the queue.
    #[test]
    fn wake_n_zero_is_noop() {
        let mut t = FutexTable::new();
        t.enqueue(0x1000, 7);
        t.enqueue(0x1000, 11);
        let woken = t.wake_n(0x1000, 0);
        assert_eq!(woken, alloc::vec![] as alloc::vec::Vec<ProcessId>);
        assert_eq!(t.peek_waiters(0x1000), &[7, 11]);
    }

    /// Round-trip: enqueue a waiter, wake exactly one, queue is empty.
    /// The headline behaviour the syscall handler depends on.
    #[test]
    fn enqueue_then_wake_one_round_trips() {
        let mut t = FutexTable::new();
        t.enqueue(0x4040, 42);
        assert_eq!(t.peek_waiters(0x4040), &[42]);
        let woken = t.wake_n(0x4040, 1);
        assert_eq!(woken, alloc::vec![42]);
        assert!(t.is_empty());
    }

    /// `wake_n` on one uaddr leaves other uaddrs untouched.
    #[test]
    fn wake_n_isolates_uaddrs() {
        let mut t = FutexTable::new();
        t.enqueue(0x1000, 7);
        t.enqueue(0x2000, 11);
        let woken = t.wake_n(0x1000, 1);
        assert_eq!(woken, alloc::vec![7]);
        // 0x2000's waiter is still parked.
        assert_eq!(t.peek_waiters(0x2000), &[11]);
        assert_eq!(t.live_uaddr_count(), 1);
    }

    /// Global singleton round-trip: `with_futex_table` sees the same
    /// state across calls. The closure shape gives transient borrow;
    /// the underlying state persists.
    #[test]
    fn with_futex_table_persists_state_across_calls() {
        // Defensive: drain anything a prior test left behind.
        with_futex_table(|t| {
            // Drain every uaddr with a high cap; quickly walks the
            // empty / mostly-empty table.
            let live: alloc::vec::Vec<u64> = t.queues.keys().copied().collect();
            for uaddr in live {
                t.wake_n(uaddr, usize::MAX);
            }
        });
        with_futex_table(|t| t.enqueue(0xdead, 99));
        let waiters: alloc::vec::Vec<ProcessId> =
            with_futex_table(|t| t.peek_waiters(0xdead).to_vec());
        assert_eq!(waiters, alloc::vec![99]);
        // Drain so the next test sees an empty global.
        with_futex_table(|t| {
            t.wake_n(0xdead, usize::MAX);
        });
    }
}
