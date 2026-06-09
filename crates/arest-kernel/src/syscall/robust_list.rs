// crates/arest-kernel/src/syscall/robust_list.rs
//
// Linux x86_64 syscalls 273 (`set_robust_list`) + 274
// (`get_robust_list`) plus the kernel-side robust-futex recovery walk
// that runs when a thread dies. Per #546 (#474c) — the owner-death
// recovery half of the PI/robust-mutex story #547 started.
//
// What a robust mutex is for
// --------------------------
// A `PTHREAD_MUTEX_ROBUST` mutex lets a program survive a thread dying
// while holding the lock: instead of every other thread deadlocking
// forever, the next acquirer gets `EOWNERDEAD`, learns the protected
// data may be inconsistent, and runs recovery
// (`pthread_mutex_consistent`). For that to work the *kernel* must,
// when a thread exits (cleanly or via a fatal signal), find every
// robust mutex the thread still held and stamp `FUTEX_OWNER_DIED` on
// each — because a dead thread can't release its own locks.
//
// The robust list
// ---------------
// Userspace can't hand the kernel a list of "locks I hold" on every
// acquire (too slow), so glibc/musl maintain a per-thread intrusive
// linked list in *userspace* memory and register its head ONCE via
// `set_robust_list(2)`. Each locked robust mutex links itself onto the
// list; unlocking unlinks it. On thread death the kernel walks the
// registered list and does the stamping. The list is deliberately in
// user memory so acquire/release stay lock-free; the cost is the
// kernel must tolerate a *corrupt* list (a thread that died mid-edit),
// which is why the walk is defensive (bounded length, a `pending`
// slot for the half-linked node, no faults on bad pointers).
//
// struct robust_list_head (LP64 layout, the only ABI tier-1 targets)
// ------------------------------------------------------------------
//   offset 0   struct robust_list list   — `.next`: head of the list.
//                                           A node's `.next` is its
//                                           first member, so a node
//                                           pointer doubles as a
//                                           `*next`. The list is
//                                           circular: the last node's
//                                           `.next` points back at
//                                           `&head->list` (== `head`,
//                                           since `list` is at offset 0).
//   offset 8   long futex_offset         — signed byte offset from a
//                                           list node to that mutex's
//                                           futex word (`_m_lock`).
//                                           `futex_word = node +
//                                           futex_offset`.
//   offset 16  void *list_op_pending      — the node a thread was in the
//                                           middle of linking/unlinking
//                                           when it died (the
//                                           list-edit + lock-acquire
//                                           aren't atomic). Walked last
//                                           so a half-linked node isn't
//                                           missed or double-processed.
// `set_robust_list` requires `len == sizeof(struct robust_list_head)`
// = `3*sizeof(long)` = 24 (musl asserts the same at
// `vendor/musl/src/thread/pthread_create.c:161`).
//
// The PI low bit
// --------------
// Robust-list `next` pointers (and `list_op_pending`) carry a flag in
// bit 0 marking the node as a PI mutex; the real pointer is `value &
// ~1`. Linux's `fetch_robust_entry` masks it; we mirror that in
// `fetch_entry` so the futex-word address is computed from the masked
// pointer.
//
// The walk algorithm (mirrors linux/kernel/futex/core.c exit_robust_list)
// ----------------------------------------------------------------------
//   1. entry        := head->list.next        (masked)
//      futex_offset := head->futex_offset
//      pending      := head->list_op_pending  (masked)
//   2. while entry != head (bounded by ROBUST_LIST_LIMIT):
//        next := entry->next                  (fetch BEFORE processing —
//                                              processing writes the futex
//                                              word, and Linux fetches the
//                                              link first defensively)
//        if entry != pending:
//            handle_futex_death(entry + futex_offset)
//        entry := next
//   3. if pending != 0: handle_futex_death(pending + futex_offset)
//
// handle_futex_death (the per-mutex stamp)
// ----------------------------------------
//   read word; if (word & FUTEX_TID_MASK) != dying_tid → leave it (the
//   thread doesn't own this lock). Else write `(word & FUTEX_WAITERS) |
//   FUTEX_OWNER_DIED` — clear the dead owner's TID, set OWNER_DIED,
//   PRESERVE the waiters bit — and, if a waiter was parked, wake exactly
//   one so it can take over and run recovery. This is the kernel's
//   careful form (preserve WAITERS); musl's *userspace* fallback at
//   `pthread_create.c:132` swaps in a bare `0x40000000` because there it
//   knows it is the owner. The futex word write + the targeted wake
//   reuse #547's `futex::write_u32` + the kernel-wide `futex_table`.
//
// Tier-1 scope
// ------------
// One thread per process (#530 brings real threads), so the robust
// list lives on the `Process` (`robust_list_head` / `robust_list_len`)
// like `fs_base`. The exit-time walk fires from `syscall::exit::
// mark_exited` (voluntary exit — the common case) and from
// `Process::deliver_signal` on a `Killed` transition (fatal-signal
// death), the two paths Linux funnels through `do_exit`. Memory access
// is the same direct deref the rest of the syscall surface uses under
// the UEFI identity mapping (see `futex::read_u32`); #561's
// copy_from_user routes it through the page tables once #552 lands
// ring 3.

use crate::process::current_process_mut;
use crate::process::futex_table::with_futex_table;
use crate::syscall::futex::{read_u32, write_u32, FUTEX_OWNER_DIED, FUTEX_TID_MASK, FUTEX_WAITERS};

/// Linux errno "No such process" — `get_robust_list` for an unknown
/// pid. `<asm-generic/errno-base.h>:ESRCH`.
pub const ESRCH: i64 = 3;

/// Linux errno "Bad address" — a null out-pointer to `get_robust_list`.
/// `<asm-generic/errno-base.h>:EFAULT`.
pub const EFAULT: i64 = 14;

/// Linux errno "Invalid argument" — `set_robust_list` with a `len`
/// other than `sizeof(struct robust_list_head)`.
/// `<asm-generic/errno-base.h>:EINVAL`.
pub const EINVAL: i64 = 22;

/// Linux errno "Function not implemented" — placeholder during the RED
/// phase; the real handlers never return it.
pub const ENOSYS: i64 = 38;

/// `sizeof(struct robust_list_head)` on LP64 = `3 * sizeof(long)` = 24.
/// `set_robust_list` rejects any other length (matches the kernel's
/// `len != sizeof(*head)` check and musl's `3*sizeof(long)` call).
pub const ROBUST_LIST_HEAD_SIZE: u64 = 24;

/// Upper bound on nodes the death-walk visits before bailing — guards
/// against a circular / corrupt list left by a thread that died
/// mid-edit. Matches Linux's `ROBUST_LIST_LIMIT`.
pub const ROBUST_LIST_LIMIT: usize = 2048;

/// Byte offset of `robust_list_head.list.next` (the list head pointer).
/// `list` is the first member, `.next` its first member → offset 0.
const OFF_LIST_NEXT: u64 = 0;
/// Byte offset of `robust_list_head.futex_offset` (a `long`).
const OFF_FUTEX_OFFSET: u64 = 8;
/// Byte offset of `robust_list_head.list_op_pending`.
const OFF_LIST_OP_PENDING: u64 = 16;
/// Bit 0 of a robust-list `next` / `list_op_pending` pointer is a PI
/// flag; the real pointer is `value & !1` (Linux `fetch_robust_entry`).
const ROBUST_PI_BIT: u64 = 1;

/// Decide the replacement word for a robust futex when its owner dies.
/// Returns `Some(new_word)` when `dying_tid` owns the lock — the new
/// word clears the owner TID, sets `FUTEX_OWNER_DIED`, and PRESERVES
/// `FUTEX_WAITERS` (Linux: `mval = (uval & FUTEX_WAITERS) |
/// FUTEX_OWNER_DIED`). Returns `None` when the word isn't owned by the
/// dying thread (an unlocked word, or one held by someone else), or
/// when `dying_tid` is 0 (no real thread has TID 0; tier-1's
/// placeholder pid 0 owns nothing). Pure — the caller does the write.
pub fn death_word(word: u32, dying_tid: u32) -> Option<u32> {
    let owner = word & FUTEX_TID_MASK;
    // An unlocked word (owner 0) is held by no one; a TID of 0 is never
    // a real owner (tier-1's placeholder pid 0 owns nothing). Either way
    // there's nothing to recover.
    if owner == 0 || dying_tid == 0 || owner != dying_tid {
        return None;
    }
    Some((word & FUTEX_WAITERS) | FUTEX_OWNER_DIED)
}

/// `set_robust_list(struct robust_list_head *head, size_t len)` —
/// register the calling thread's robust-list head. `len` must equal
/// `sizeof(struct robust_list_head)` (24) or the call is `-EINVAL`. A
/// `head` of 0 de-registers the list (Linux accepts it). Stores the
/// pair on the current Process; returns 0.
pub fn set_robust_list(head: u64, len: u64) -> i64 {
    // The length check is the kernel's first gate (`len != sizeof(*head)
    // → -EINVAL`) and fires before any process touch, so it's honest
    // even pre-init.
    if len != ROBUST_LIST_HEAD_SIZE {
        return -EINVAL;
    }
    current_process_mut(|maybe| {
        if let Some(proc) = maybe {
            proc.robust_list_head = head;
            proc.robust_list_len = len;
        }
    });
    0
}

/// `get_robust_list(int pid, struct robust_list_head **head_ptr,
/// size_t *len_ptr)` — report the robust-list head + len thread `pid`
/// registered. `pid == 0` means the caller. Tier-1 has one thread, so
/// only pid 0 / the current pid is valid (else `-ESRCH`); a null
/// `head_ptr` / `len_ptr` is `-EFAULT`. Writes the stored head + len
/// through the out-pointers; returns 0.
pub fn get_robust_list(pid: u64, head_ptr: u64, len_ptr: u64) -> i64 {
    if head_ptr == 0 || len_ptr == 0 {
        return -EFAULT;
    }
    let info =
        current_process_mut(|maybe| maybe.map(|p| (p.pid, p.robust_list_head, p.robust_list_len)));
    let (head, len) = match info {
        Some((cur_pid, head, len)) => {
            // `pid == 0` means "the calling thread"; otherwise it must
            // name the (single, tier-1) current thread.
            if pid != 0 && pid != cur_pid as u64 {
                return -ESRCH;
            }
            (head, len)
        }
        None => {
            // No current thread (tier-1 pre-init). Only "self" (pid 0)
            // is meaningful; report an empty registration.
            if pid != 0 {
                return -ESRCH;
            }
            (0, 0)
        }
    };
    write_u64(head_ptr, head);
    write_u64(len_ptr, len);
    0
}

/// Walk a dying thread's robust list and stamp `FUTEX_OWNER_DIED` on
/// every mutex it still owns, waking one recovery waiter per stamped
/// mutex. `head` is the userspace VA from `set_robust_list`; `len` the
/// registered byte length (must be `ROBUST_LIST_HEAD_SIZE` to walk —
/// a malformed registration is skipped); `dying_tid` the exiting
/// thread's TID. A `head` of 0, a bad `len`, or `dying_tid == 0` is a
/// no-op.
pub fn walk_on_death(head: u64, len: u64, dying_tid: u32) {
    // No list registered, a malformed registration, or no real owner →
    // nothing to recover.
    if head == 0 || len != ROBUST_LIST_HEAD_SIZE || dying_tid == 0 {
        return;
    }
    let futex_offset = read_u64(head + OFF_FUTEX_OFFSET) as i64;
    let pending = fetch_entry(head + OFF_LIST_OP_PENDING);
    let mut entry = fetch_entry(head + OFF_LIST_NEXT);
    let mut count = 0usize;
    // The list is circular: the last node's `.next` points back at
    // `&head->list` (== `head`). Stop there, or after ROBUST_LIST_LIMIT
    // nodes if the list is corrupt / circular elsewhere.
    while entry != head && count < ROBUST_LIST_LIMIT {
        // Fetch the link before processing — Linux reads `next_entry`
        // first (defensive against a node that frees itself mid-walk).
        let next = fetch_entry(entry + OFF_LIST_NEXT);
        // The pending node is handled once, after the loop — skip it
        // here so it isn't double-processed.
        if entry != pending {
            handle_futex_death(futex_word_addr(entry, futex_offset), dying_tid);
        }
        entry = next;
        count += 1;
    }
    // The lock a thread was mid-acquire/release on when it died: its
    // list link may be half-written, so the kernel tracks it separately
    // and stamps it last.
    if pending != 0 {
        handle_futex_death(futex_word_addr(pending, futex_offset), dying_tid);
    }
}

/// `node + futex_offset` (signed) — the address of the mutex's futex
/// word given a list node pointer. `futex_offset` can be negative (the
/// futex word may sit before the list link in the mutex struct).
fn futex_word_addr(entry: u64, futex_offset: i64) -> u64 {
    (entry as i64).wrapping_add(futex_offset) as u64
}

/// Read a robust-list pointer and mask off its PI flag bit (Linux
/// `fetch_robust_entry`).
fn fetch_entry(addr: u64) -> u64 {
    read_u64(addr) & !ROBUST_PI_BIT
}

/// Stamp `FUTEX_OWNER_DIED` on one robust futex word if `dying_tid`
/// owns it, waking one recovery waiter when the word had waiters
/// parked. Skips a null / misaligned word rather than faulting — the
/// list may be corrupt.
fn handle_futex_death(uaddr: u64, dying_tid: u32) {
    if uaddr == 0 || uaddr & 0b11 != 0 {
        return;
    }
    let word = read_u32(uaddr);
    if let Some(new_word) = death_word(word, dying_tid) {
        write_u32(uaddr, new_word);
        // A parked waiter must be woken so it can acquire the now-dead
        // lock and learn (via -EOWNERDEAD on its LOCK_PI) it owns
        // possibly-inconsistent state. Wake exactly one (hand-off).
        if word & FUTEX_WAITERS != 0 {
            with_futex_table(|t| {
                t.wake_n(uaddr, 1);
            });
        }
    }
}

/// Read a 4-byte-aligned-or-not 8-byte little-endian value from a
/// userspace VA — the robust-list pointer / offset counterpart to
/// `futex::read_u32`. Direct deref under tier-1 identity mapping.
///
/// SAFETY: `walk_on_death` only derefs the head (validated non-zero) and
/// node pointers the list itself supplies. A corrupt list (a thread
/// that died mid-edit) is the same risk Linux's `exit_robust_list`
/// runs with; it `get_user()`s and bails on fault, but tier-1 has no
/// fault surface yet (#561 copy_from_user). `read_volatile` keeps the
/// read from being elided / reordered across the userspace boundary.
fn read_u64(addr: u64) -> u64 {
    unsafe { core::ptr::read_volatile(addr as *const u64) }
}

/// Write an 8-byte little-endian value to a userspace VA — used by
/// `get_robust_list` to fill the `head_ptr` / `len_ptr` out-params.
///
/// SAFETY: `get_robust_list` validated both out-pointers non-null; the
/// identity mapping makes the userspace VA a kernel VA (the
/// `futex::write_u32` contract).
fn write_u64(addr: u64, value: u64) {
    unsafe { core::ptr::write_volatile(addr as *mut u64, value) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::futex_table::with_futex_table;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::{
        current_process_install, current_process_mut, current_process_uninstall, Process,
    };

    // ---- struct layouts the walk tests build in host memory ----------
    //
    // A "mutex" is a list node (`next`) immediately followed by its
    // futex word (`lock`). `futex_offset` is therefore
    // `offsetof(lock) - offsetof(next)` = 8 (next: u64 @0, lock: u32 @8).
    // 8-byte alignment keeps `lock` 4-byte aligned (handle_futex_death
    // skips misaligned words).
    #[repr(C, align(8))]
    struct TestMutex {
        next: u64,
        lock: u32,
        _pad: u32,
    }
    #[repr(C, align(8))]
    struct TestHead {
        list_next: u64,
        futex_offset: i64,
        pending: u64,
    }
    const FUTEX_OFFSET: i64 = 8;

    fn addr<T>(t: &T) -> u64 {
        t as *const T as u64
    }

    fn install_test_process(pid: u32) {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(pid, address_space);
        current_process_install(proc);
    }

    // ===== death_word (pure) ==========================================

    /// A word owned by the dying thread becomes `OWNER_DIED` with the
    /// TID cleared and no waiters bit (none was set).
    #[test]
    fn death_word_owned_no_waiters_sets_owner_died() {
        assert_eq!(death_word(0x1234, 0x1234), Some(FUTEX_OWNER_DIED));
    }

    /// A word owned by the dying thread WITH waiters keeps the waiters
    /// bit and adds OWNER_DIED (Linux: `(uval & FUTEX_WAITERS) |
    /// FUTEX_OWNER_DIED`).
    #[test]
    fn death_word_owned_with_waiters_preserves_waiters() {
        let word = 0x1234 | FUTEX_WAITERS;
        assert_eq!(
            death_word(word, 0x1234),
            Some(FUTEX_WAITERS | FUTEX_OWNER_DIED)
        );
    }

    /// A word owned by a DIFFERENT live thread is left alone.
    #[test]
    fn death_word_owned_by_other_is_none() {
        assert_eq!(death_word(0x1234, 0x9999), None);
    }

    /// An unlocked word (owner TID 0) is owned by no one — never stamped,
    /// even if the dying tid were somehow 0.
    #[test]
    fn death_word_unlocked_is_none() {
        assert_eq!(death_word(0, 5), None);
        assert_eq!(death_word(FUTEX_WAITERS, 5), None);
    }

    /// dying_tid 0 (tier-1 placeholder / no real thread) stamps nothing.
    #[test]
    fn death_word_dying_tid_zero_is_none() {
        assert_eq!(death_word(0x1234, 0), None);
    }

    /// An already-OWNER_DIED word is not re-owned: its TID bits are 0,
    /// so no live tid matches → None (idempotent re-walk).
    #[test]
    fn death_word_already_owner_died_is_none() {
        assert_eq!(death_word(FUTEX_OWNER_DIED, 0x1234), None);
        assert_eq!(death_word(FUTEX_OWNER_DIED | FUTEX_WAITERS, 0x1234), None);
    }

    // ===== set_robust_list / get_robust_list ==========================

    /// `set_robust_list(head, 24)` stores the head + len on the process
    /// and returns 0.
    #[test]
    fn set_robust_list_stores_and_returns_zero() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process(7);
        let r = set_robust_list(0xdead_0000, ROBUST_LIST_HEAD_SIZE);
        assert_eq!(r, 0);
        current_process_mut(|m| {
            let p = m.expect("process installed");
            assert_eq!(p.robust_list_head, 0xdead_0000);
            assert_eq!(p.robust_list_len, ROBUST_LIST_HEAD_SIZE);
        });
        current_process_uninstall();
    }

    /// `set_robust_list` with a wrong `len` is `-EINVAL` and stores
    /// nothing.
    #[test]
    fn set_robust_list_wrong_len_is_einval() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process(7);
        assert_eq!(set_robust_list(0xdead_0000, 8), -EINVAL);
        assert_eq!(set_robust_list(0xdead_0000, 0), -EINVAL);
        current_process_mut(|m| {
            let p = m.expect("process installed");
            assert_eq!(p.robust_list_head, 0, "nothing stored on EINVAL");
        });
        current_process_uninstall();
    }

    /// `get_robust_list(0, &head, &len)` round-trips what
    /// `set_robust_list` stored.
    #[test]
    fn get_robust_list_round_trips_set() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process(7);
        set_robust_list(0xbeef_0000, ROBUST_LIST_HEAD_SIZE);

        let mut out_head: u64 = 0;
        let mut out_len: u64 = 0;
        let r = get_robust_list(0, addr(&out_head), addr(&out_len));
        assert_eq!(r, 0);
        assert_eq!(out_head, 0xbeef_0000);
        assert_eq!(out_len, ROBUST_LIST_HEAD_SIZE);
        current_process_uninstall();
    }

    /// `get_robust_list` with a null out-pointer is `-EFAULT`.
    #[test]
    fn get_robust_list_null_out_is_efault() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process(7);
        let mut out: u64 = 0;
        assert_eq!(get_robust_list(0, 0, addr(&out)), -EFAULT);
        assert_eq!(get_robust_list(0, addr(&out), 0), -EFAULT);
        current_process_uninstall();
    }

    /// `get_robust_list` for a pid that isn't the caller is `-ESRCH`
    /// (tier-1 has a single thread).
    #[test]
    fn get_robust_list_unknown_pid_is_esrch() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process(7);
        let mut out_head: u64 = 0;
        let mut out_len: u64 = 0;
        assert_eq!(
            get_robust_list(999, addr(&out_head), addr(&out_len)),
            -ESRCH
        );
        current_process_uninstall();
    }

    // ===== walk_on_death (host-memory linked lists) ===================

    /// A single owned mutex on the list gets `OWNER_DIED` stamped; the
    /// TID is cleared.
    #[test]
    fn walk_single_owned_mutex_stamps_owner_died() {
        let mut m = TestMutex { next: 0, lock: 7, _pad: 0 };
        let mut head = TestHead { list_next: 0, futex_offset: FUTEX_OFFSET, pending: 0 };
        // head -> m -> head (circular terminator).
        m.next = addr(&head);
        head.list_next = addr(&m);

        walk_on_death(addr(&head), ROBUST_LIST_HEAD_SIZE, 7);

        let lock = read_u32(addr(&m) + 8);
        assert_eq!(lock, FUTEX_OWNER_DIED, "TID cleared, OWNER_DIED set");
    }

    /// A mutex owned by a DIFFERENT thread is left untouched.
    #[test]
    fn walk_mutex_owned_by_other_untouched() {
        let mut m = TestMutex { next: 0, lock: 99, _pad: 0 };
        let mut head = TestHead { list_next: 0, futex_offset: FUTEX_OFFSET, pending: 0 };
        m.next = addr(&head);
        head.list_next = addr(&m);

        walk_on_death(addr(&head), ROBUST_LIST_HEAD_SIZE, 7);

        assert_eq!(read_u32(addr(&m) + 8), 99, "other owner's lock untouched");
    }

    /// Two mutexes on the list are both stamped.
    #[test]
    fn walk_two_owned_mutexes_both_stamped() {
        let mut m1 = TestMutex { next: 0, lock: 7, _pad: 0 };
        let mut m2 = TestMutex { next: 0, lock: 7, _pad: 0 };
        let mut head = TestHead { list_next: 0, futex_offset: FUTEX_OFFSET, pending: 0 };
        // head -> m1 -> m2 -> head.
        head.list_next = addr(&m1);
        m1.next = addr(&m2);
        m2.next = addr(&head);

        walk_on_death(addr(&head), ROBUST_LIST_HEAD_SIZE, 7);

        assert_eq!(read_u32(addr(&m1) + 8), FUTEX_OWNER_DIED);
        assert_eq!(read_u32(addr(&m2) + 8), FUTEX_OWNER_DIED);
    }

    /// The `list_op_pending` node (not on the main list) is stamped too —
    /// the lock a thread was mid-acquire on when it died.
    #[test]
    fn walk_pending_node_is_stamped() {
        let mut mp = TestMutex { next: 0, lock: 7, _pad: 0 };
        // Empty main list: head.list_next points back at head.
        let mut head = TestHead { list_next: 0, futex_offset: FUTEX_OFFSET, pending: 0 };
        head.list_next = addr(&head);
        head.pending = addr(&mp);
        mp.next = 0;

        walk_on_death(addr(&head), ROBUST_LIST_HEAD_SIZE, 7);

        assert_eq!(read_u32(addr(&mp) + 8), FUTEX_OWNER_DIED, "pending stamped");
    }

    /// A node that is BOTH on the list and the pending node is processed
    /// exactly once (skipped in the loop, handled at the end) — stamping
    /// is idempotent so the observable result is a single OWNER_DIED.
    #[test]
    fn walk_pending_also_on_list_processed_once() {
        let mut m = TestMutex { next: 0, lock: 7, _pad: 0 };
        let mut head = TestHead { list_next: 0, futex_offset: FUTEX_OFFSET, pending: 0 };
        head.list_next = addr(&m);
        m.next = addr(&head);
        head.pending = addr(&m); // same node is pending

        walk_on_death(addr(&head), ROBUST_LIST_HEAD_SIZE, 7);

        assert_eq!(read_u32(addr(&m) + 8), FUTEX_OWNER_DIED);
    }

    /// The PI low-bit on a `next` pointer is masked off before computing
    /// the futex-word address (Linux `fetch_robust_entry`).
    #[test]
    fn walk_masks_pi_low_bit_on_entry() {
        let mut m = TestMutex { next: 0, lock: 7, _pad: 0 };
        let mut head = TestHead { list_next: 0, futex_offset: FUTEX_OFFSET, pending: 0 };
        m.next = addr(&head);
        // Set the PI flag bit on the head's pointer to the node.
        head.list_next = addr(&m) | 1;

        walk_on_death(addr(&head), ROBUST_LIST_HEAD_SIZE, 7);

        assert_eq!(read_u32(addr(&m) + 8), FUTEX_OWNER_DIED, "PI bit masked");
    }

    /// `head == 0` (no list registered) is a no-op — and doesn't fault.
    #[test]
    fn walk_null_head_is_noop() {
        walk_on_death(0, ROBUST_LIST_HEAD_SIZE, 7);
        // No panic / fault == pass.
    }

    /// A wrong `len` (malformed registration) skips the walk entirely,
    /// even if the head looks valid.
    #[test]
    fn walk_wrong_len_is_noop() {
        let mut m = TestMutex { next: 0, lock: 7, _pad: 0 };
        let mut head = TestHead { list_next: 0, futex_offset: FUTEX_OFFSET, pending: 0 };
        m.next = addr(&head);
        head.list_next = addr(&m);

        walk_on_death(addr(&head), 8, 7); // wrong len

        assert_eq!(read_u32(addr(&m) + 8), 7, "not walked → lock unchanged");
    }

    /// A self-referential (circular, never-reaches-head) list terminates
    /// via ROBUST_LIST_LIMIT rather than looping forever — and the node
    /// still ends up stamped (idempotently).
    #[test]
    fn walk_circular_list_terminates() {
        let mut m = TestMutex { next: 0, lock: 7, _pad: 0 };
        let mut head = TestHead { list_next: 0, futex_offset: FUTEX_OFFSET, pending: 0 };
        head.list_next = addr(&m);
        m.next = addr(&m); // points at itself — never == head

        walk_on_death(addr(&head), ROBUST_LIST_HEAD_SIZE, 7);

        // If we got here, the walk terminated (didn't hang).
        assert_eq!(read_u32(addr(&m) + 8), FUTEX_OWNER_DIED);
    }

    /// When the dying owner held a mutex WITH waiters parked, the walk
    /// stamps OWNER_DIED *and* wakes exactly one waiter so it can take
    /// over and run recovery.
    #[test]
    fn walk_with_waiters_wakes_one() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let mut m = TestMutex { next: 0, lock: 7 | FUTEX_WAITERS, _pad: 0 };
        let mut head = TestHead { list_next: 0, futex_offset: FUTEX_OFFSET, pending: 0 };
        m.next = addr(&head);
        head.list_next = addr(&m);
        let lock_addr = addr(&m) + 8;

        // Park two waiters on the lock's futex address; the walk should
        // drain exactly one.
        with_futex_table(|t| {
            t.wake_n(lock_addr, usize::MAX); // clean slate for this addr
            t.enqueue(lock_addr, 21);
            t.enqueue(lock_addr, 22);
        });

        walk_on_death(addr(&head), ROBUST_LIST_HEAD_SIZE, 7);

        // OWNER_DIED set, waiters bit preserved.
        assert_eq!(read_u32(lock_addr), FUTEX_WAITERS | FUTEX_OWNER_DIED);
        // Exactly one waiter drained (FIFO: 21 woke, 22 remains).
        let remaining: alloc::vec::Vec<u32> =
            with_futex_table(|t| t.peek_waiters(lock_addr).to_vec());
        assert_eq!(remaining, alloc::vec![22], "one waiter woken for recovery");

        with_futex_table(|t| {
            t.wake_n(lock_addr, usize::MAX);
        });
    }
}
