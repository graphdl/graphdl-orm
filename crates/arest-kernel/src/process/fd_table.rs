// crates/arest-kernel/src/process/fd_table.rs
//
// Per-process file-descriptor table (#498 Track KKKKK, second leg of
// the userspace-syscall epic #473). The first slice (#507, GGGGG)
// shipped `write` + `exit` against the seed `Vec<FdEntry>` carried on
// `Process` — three slots (stdin / stdout / stderr) all backed by the
// kernel's serial console. That seeded table is enough for a "hello
// world" binary that only writes(1) but it can't model the result of
// an `open(2)` / `openat(2)` because the entries themselves only carry
// `Serial` / `Closed` variants.
//
// What this module ships
// ----------------------
// A richer per-process fd table — `FdTable { entries: BTreeMap<i32,
// FdEntry> }` — that the `openat` (#498, this slice) and `close`
// (this slice) handlers manipulate. Two backing variants today:
//
//   * `FdEntry::File { cell_id }` — fd backed by a File entity in the
//     cell graph (#398). The cell_id is the AREST id (e.g. the hex
//     blob hash) the `File_has_*` facts are keyed on; the future
//     `read` handler (#499) will look up `File_has_ContentRef` against
//     this id to source bytes.
//   * `FdEntry::Synthetic { path }` — fd backed by the synthetic-fs
//     resolver (HHHHH's #534 — `/proc/cpuinfo`, `/proc/meminfo`, the
//     future `/sys/*` and `/dev/*` entries). The path is the absolute
//     POSIX-style path the resolver matches; the future `read` handler
//     calls `synthetic_fs::resolve(path)` to source bytes.
//
// Per-process containment
// -----------------------
// Linux's fd table is per-process — fds are NOT shared across
// processes (modulo `clone(CLONE_FILES)` / `vfork` corner cases that
// tier-1 doesn't model). The table lives on the `Process` struct
// alongside the rest of the per-process state. The accessor
// (`process::current_process_fd_table`) mirrors GGGGG's
// `current_process_mut` shape — closure-style, single-threaded
// kernel — so the future scheduler (#530) can swap the
// `CURRENT_PROCESS` static without re-shaping the call site.
//
// fd allocation policy
// --------------------
// POSIX requires `open(2)` / `openat(2)` to return the lowest-numbered
// free fd ≥ 0. The seeded fds 0/1/2 are reserved for the standard
// streams; new allocations start at fd 3 and walk up looking for a
// gap (close(fd) returns the slot to the free pool, dup2(fd, ...) can
// punch a hole). Tier-1 caps the fd space at 1024 entries per process
// (Linux's default `RLIMIT_NOFILE` soft limit) — exhausting the pool
// returns an error, mapped by the `openat` handler to `-EMFILE`. A
// future setrlimit / prlimit surface (#531-followups) will widen the
// cap; for tier-1 the constant is enough.
//
// Why BTreeMap rather than Vec
// ----------------------------
// The fd space is sparse — `dup2(2)` and `fcntl(F_DUPFD_CLOEXEC, ...)`
// can punch holes at arbitrary indices, and a future `select(2)` /
// `poll(2)` surface needs efficient "iterate every open fd" without
// walking a Vec full of `Closed` placeholders. `BTreeMap<i32, FdEntry>`
// keeps the storage proportional to the number of OPEN fds, gives O(log
// n) lookup / insert / remove, and walks in fd-numerical order which
// is what `select` wants. The cost is one heap allocation per entry vs
// Vec's amortised zero — fine because fd open / close is a low-rate
// event compared to read / write.
//
// Errors
// ------
// `release(fd)` returns `Err(())` for an unknown fd — the close handler
// translates that to `-EBADF`. `allocate` returns `Err(())` when the
// 1024-entry cap is hit — the openat handler translates that to
// `-EMFILE`. Bounded errors stay as `Result<_, ()>` rather than a richer
// enum because the only consumer (the syscall handlers) maps each to a
// fixed errno; a richer error type would just round-trip through a
// match without adding information.
//
// What this module does NOT do (intentionally — see #473 epic):
//   * `read(fd, ...)` — #499 (next slice). This module just stores
//     enough state for the read handler to source bytes; it does not
//     itself implement the byte-pull.
//   * `dup` / `dup2` / `dup3` / `fcntl(F_DUPFD)` — those grow the
//     allocation surface (a target fd, not just lowest-free); the
//     foundation here is enough to add them as `allocate_at(fd)` /
//     `dup(fd) -> i32` follow-ups.
//   * `close-on-exec` (`O_CLOEXEC`, `FD_CLOEXEC`) — once `execve(2)`
//     lands (#560-followups), the table grows a per-entry CLOEXEC bit
//     that gets walked on exec.
//   * Per-fd offset (the `lseek(2)` cursor). The `read` handler will
//     introduce this when it lands — the `FdEntry` variants gain an
//     offset field at that point. Today there's no `read` so no
//     offset is needed.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};

/// Lowest fd a `FdTable::allocate` call will hand out. Per POSIX
/// `open(2)` returns the lowest-numbered free fd; tier-1 reserves
/// 0 / 1 / 2 for stdin / stdout / stderr (which the seeded `Process`
/// fd_table — the legacy `Vec<FdEntry>` carried alongside this richer
/// table — already routes through GGGGG's `write` handler). New
/// open()s start at 3.
pub const FIRST_USER_FD: i32 = 3;

/// Soft cap on per-process open fds. Matches Linux's default
/// `RLIMIT_NOFILE` soft limit (1024). When the table is full,
/// `allocate` returns `Err(())` and the `openat` handler maps that
/// to `-EMFILE`. A future `setrlimit` surface (#531-followups) will
/// widen the cap; tier-1 keeps it constant.
pub const MAX_OPEN_FDS: usize = 1024;

/// Per-fd state — what kind of resource a given fd is open against.
/// Kept small (one tag + one heap allocation per variant) because the
/// table can hold up to 1024 entries per process. Future variants
/// (Socket, Pipe, EventFd) plug in here without reshaping the table
/// itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FdEntry {
    /// fd backed by a File entity in the cell graph (#398). The
    /// `cell_id` is the AREST id (typically the hex blob hash) the
    /// `File_has_*` facts are keyed on. The future `read` handler
    /// (#499) walks `File_has_ContentRef` against this id to source
    /// bytes; the future `write` handler (post-#499) walks the same
    /// fact to source the destination.
    File {
        /// AREST cell id keying the `File_has_*` facts.
        cell_id: String,
    },
    /// fd backed by the synthetic-fs resolver (HHHHH's #534 —
    /// `/proc/cpuinfo`, `/proc/meminfo` today; the future `/sys/*` and
    /// `/dev/*` entries from #535-#537 plug in transparently). The
    /// `path` is the absolute POSIX-style path the resolver matches
    /// — the future `read` handler calls `synthetic_fs::resolve(path)`
    /// to source bytes. Synthetic entries are read-only by construction;
    /// the `openat` handler rejects `O_WRONLY` / `O_RDWR` against them
    /// with `-EACCES`.
    Synthetic {
        /// Absolute POSIX-style path the synthetic resolver matches.
        path: String,
    },
}

/// Per-process file-descriptor table. Owns a sparse `BTreeMap` keyed by
/// fd number — fds 0 / 1 / 2 are reserved for the standard streams (the
/// legacy `Vec<FdEntry>` on `Process` already routes those through
/// GGGGG's write handler), so this table only ever populates entries at
/// fd ≥ 3.
///
/// Constructed via `FdTable::new` (empty); mutated via `allocate` /
/// `release`. Lookup is `lookup(fd) -> Option<&FdEntry>`.
#[derive(Debug, Default)]
pub struct FdTable {
    entries: BTreeMap<i32, FdEntry>,
}

impl FdTable {
    /// Construct an empty fd table. fds 0 / 1 / 2 are NOT seeded here
    /// — the legacy `Vec<FdEntry>` on `Process::new` carries them
    /// instead, and GGGGG's write handler reaches them via that
    /// surface. This table is the open()-side state introduced by
    /// `openat` (#498).
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Allocate the lowest-numbered free fd ≥ `FIRST_USER_FD` and bind
    /// it to `entry`. Returns the allocated fd on success, `Err(())`
    /// when the 1024-entry cap is hit (the `openat` handler maps that
    /// to `-EMFILE`).
    ///
    /// Walks the existing entries in fd-numerical order looking for the
    /// first gap. The walk is O(n) in the number of OPEN fds — which is
    /// fine for tier-1 (typical process holds <16 fds at a time). A
    /// future "free list" optimisation can replace the walk; the public
    /// surface stays the same.
    pub fn allocate(&mut self, entry: FdEntry) -> Result<i32, ()> {
        if self.entries.len() >= MAX_OPEN_FDS {
            return Err(());
        }
        let mut next = FIRST_USER_FD;
        for &existing in self.entries.keys() {
            if existing > next {
                // Found a gap before `existing`.
                break;
            }
            if existing == next {
                next = next.checked_add(1).ok_or(())?;
            }
            // existing < next means a stale fd below the cursor; the
            // BTreeMap's ordered keys guarantee this doesn't happen
            // because we walked from the smallest key upward — but the
            // arm is intentionally a no-op rather than `unreachable!()`
            // so a future relaxed invariant doesn't kernel-panic.
        }
        self.entries.insert(next, entry);
        Ok(next)
    }

    /// Look up `fd`'s backing entry. Returns `None` for an unknown fd
    /// (the read / write handlers map that to `-EBADF`).
    pub fn lookup(&self, fd: i32) -> Option<&FdEntry> {
        self.entries.get(&fd)
    }

    /// Release `fd` from the table. Returns `Err(())` when `fd` is not
    /// open (the close handler maps that to `-EBADF`). On success the
    /// fd is freed and a future `allocate` may re-issue it.
    ///
    /// Closing fds 0 / 1 / 2 is undefined here — those slots aren't in
    /// this table (the legacy `Vec<FdEntry>` on `Process` carries
    /// them). A user `close(0)` / `close(1)` / `close(2)` returns
    /// `-EBADF` from this surface, which is correct: tier-1 doesn't
    /// support closing the standard streams, and the future fd-table
    /// unification (when the legacy `Vec<FdEntry>` is folded into this
    /// type) will introduce the right behaviour.
    pub fn release(&mut self, fd: i32) -> Result<(), ()> {
        self.entries.remove(&fd).map(|_| ()).ok_or(())
    }

    /// Number of currently-open fds in this table. Excludes the
    /// standard streams (fd 0 / 1 / 2), which live on the legacy
    /// `Vec<FdEntry>`. Used by the unit tests to assert allocation /
    /// release behaviour without poking the internals.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no fds are open. Convenience for the unit tests +
    /// future `getrlimit` surface.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// `String` constructors require `alloc::string::ToString` to be in
// scope at the call site so e.g. `FdEntry::Synthetic { path:
// "/proc/cpuinfo".to_string() }` compiles. The use is internal to the
// allocate / lookup / release helpers below; re-exporting here keeps
// the call sites short.
pub fn synthetic(path: &str) -> FdEntry {
    FdEntry::Synthetic {
        path: path.to_string(),
    }
}

/// Constructor helper — wraps a cell id into `FdEntry::File`. Same
/// shape as `synthetic` above: the `String` allocation is hidden so
/// call sites can write `fd_table::file("abc123")` rather than
/// reaching for `ToString`.
pub fn file(cell_id: &str) -> FdEntry {
    FdEntry::File {
        cell_id: cell_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `FdTable::new` produces an empty table.
    #[test]
    fn new_table_is_empty() {
        let t = FdTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    /// First allocate against an empty table returns fd 3 (the lowest
    /// fd ≥ FIRST_USER_FD, leaving 0 / 1 / 2 reserved for the standard
    /// streams).
    #[test]
    fn first_allocate_returns_fd_three() {
        let mut t = FdTable::new();
        let fd = t.allocate(synthetic("/proc/cpuinfo")).expect("allocate");
        assert_eq!(fd, FIRST_USER_FD);
        assert_eq!(fd, 3);
    }

    /// Sequential allocations against an empty table grow monotonically:
    /// 3, 4, 5, ...
    #[test]
    fn sequential_allocates_walk_upward() {
        let mut t = FdTable::new();
        let fd_a = t.allocate(synthetic("/proc/cpuinfo")).expect("a");
        let fd_b = t.allocate(synthetic("/proc/meminfo")).expect("b");
        let fd_c = t.allocate(file("abc")).expect("c");
        assert_eq!(fd_a, 3);
        assert_eq!(fd_b, 4);
        assert_eq!(fd_c, 5);
    }

    /// `lookup` returns the variant the table was populated with —
    /// File for File-cell-backed fds, Synthetic for synthetic-fs paths.
    #[test]
    fn lookup_returns_populated_variant() {
        let mut t = FdTable::new();
        let fd_syn = t.allocate(synthetic("/proc/cpuinfo")).expect("syn");
        let fd_file = t.allocate(file("abc123")).expect("file");
        assert_eq!(
            t.lookup(fd_syn),
            Some(&FdEntry::Synthetic {
                path: "/proc/cpuinfo".into()
            })
        );
        assert_eq!(
            t.lookup(fd_file),
            Some(&FdEntry::File {
                cell_id: "abc123".into()
            })
        );
    }

    /// `lookup` of an unknown fd returns `None`.
    #[test]
    fn lookup_unknown_fd_returns_none() {
        let t = FdTable::new();
        assert_eq!(t.lookup(7), None);
        assert_eq!(t.lookup(0), None); // standard streams not in this table
        assert_eq!(t.lookup(-1), None);
    }

    /// `release` of an open fd removes the entry and returns `Ok(())`.
    /// Subsequent lookup returns `None`.
    #[test]
    fn release_open_fd_removes_entry() {
        let mut t = FdTable::new();
        let fd = t.allocate(synthetic("/proc/cpuinfo")).expect("alloc");
        assert!(t.lookup(fd).is_some());
        t.release(fd).expect("release");
        assert!(t.lookup(fd).is_none());
    }

    /// `release` of an unknown fd returns `Err(())` — the close handler
    /// translates that to `-EBADF`.
    #[test]
    fn release_unknown_fd_returns_err() {
        let mut t = FdTable::new();
        assert_eq!(t.release(99), Err(()));
        assert_eq!(t.release(0), Err(())); // standard streams not in this table
        assert_eq!(t.release(-1), Err(()));
    }

    /// After `release`, the freed slot is re-used by the next allocate
    /// (POSIX "lowest free fd" guarantee).
    #[test]
    fn release_then_allocate_reuses_lowest_freed_slot() {
        let mut t = FdTable::new();
        let fd_a = t.allocate(synthetic("/proc/cpuinfo")).expect("a");
        let fd_b = t.allocate(synthetic("/proc/meminfo")).expect("b");
        let fd_c = t.allocate(file("c")).expect("c");
        assert_eq!(fd_a, 3);
        assert_eq!(fd_b, 4);
        assert_eq!(fd_c, 5);
        // Free the middle slot.
        t.release(fd_b).expect("release b");
        // Next allocate should re-use fd 4 (the lowest free fd ≥ 3).
        let fd_d = t.allocate(file("d")).expect("d");
        assert_eq!(fd_d, 4);
    }

    /// Allocations cap at MAX_OPEN_FDS entries; the 1025th allocate
    /// returns `Err(())`. The openat handler maps this to `-EMFILE`.
    #[test]
    fn allocate_at_cap_returns_err() {
        let mut t = FdTable::new();
        for _ in 0..MAX_OPEN_FDS {
            t.allocate(synthetic("/proc/cpuinfo")).expect("under cap");
        }
        assert_eq!(t.len(), MAX_OPEN_FDS);
        assert!(t.allocate(synthetic("/proc/meminfo")).is_err());
    }

    /// `release` of fds 0 / 1 / 2 returns `Err(())` — those slots are
    /// not in this table (the legacy `Vec<FdEntry>` on `Process`
    /// carries them). Documents the tier-1 limitation.
    #[test]
    fn release_standard_stream_fd_returns_err_under_tier_1() {
        let mut t = FdTable::new();
        assert_eq!(t.release(0), Err(()));
        assert_eq!(t.release(1), Err(()));
        assert_eq!(t.release(2), Err(()));
    }

    /// `synthetic` helper produces an `FdEntry::Synthetic` with the
    /// expected path. Documents the constructor's behaviour.
    #[test]
    fn synthetic_helper_constructs_synthetic_variant() {
        let entry = synthetic("/proc/cpuinfo");
        assert_eq!(
            entry,
            FdEntry::Synthetic {
                path: "/proc/cpuinfo".into()
            }
        );
    }

    /// `file` helper produces an `FdEntry::File` with the expected
    /// cell id.
    #[test]
    fn file_helper_constructs_file_variant() {
        let entry = file("abc123");
        assert_eq!(
            entry,
            FdEntry::File {
                cell_id: "abc123".into()
            }
        );
    }
}
