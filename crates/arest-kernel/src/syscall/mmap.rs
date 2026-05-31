// crates/arest-kernel/src/syscall/mmap.rs
//
// Linux x86_64 syscalls 9 (`mmap`) and 11 (`munmap`). Per #497
// (anonymous mmap / malloc-backing path).
//
// Linux x86_64 numbers:
//   `__NR_mmap    =  9`   (`linux/arch/x86/include/uapi/asm/unistd_64.h`)
//   `__NR_munmap  = 11`   (same source)
//
// mmap(2) calling convention (Linux x86_64 ABI)
// -----------------------------------------------
// `mmap(addr, len, prot, flags, fd, off)` is a 6-argument syscall; the
// arguments occupy the six general-purpose argument registers in order:
//
//   rdi = addr   — requested base address (0 = "kernel picks")
//   rsi = len    — mapping length in bytes (> 0)
//   rdx = prot   — memory protection flags (PROT_*)
//   r10 = flags  — mapping flags (MAP_PRIVATE, MAP_ANONYMOUS, …)
//   r8  = fd     — file descriptor (-1 for anonymous mappings)
//   r9  = off    — file offset (0 for anonymous mappings)
//
// Note: r10, not rcx, carries the fourth argument for syscalls. The
// musl syscall wrapper confirms this at:
//   `vendor/musl/arch/x86_64/syscall_arch.h:__syscall6`
//
// Tier-1 scope: anonymous mappings only
// ----------------------------------------
// This slice implements the ANONYMOUS path (`flags & MAP_ANONYMOUS != 0`,
// `fd` ignored). File-backed mappings (MAP_ANONYMOUS clear) return
// `-ENODEV` (-19) because no VFS / block device layer exists in tier-1
// to source the page contents from.
//
// Bump-allocator strategy
// -----------------------
// Full page-table manipulation (allocating physical frames + installing
// PTEs) is deferred to the boot-integration track (#527 + future child
// tasks of #497). On this foundation slice we implement a MONOTONIC BUMP
// ALLOCATOR over a reserved virtual region:
//
//   1. `Process::mmap_bump` starts at `MMAP_BASE` (0x7000_0000_0000) —
//      a canonical mmap-region start on Linux x86_64, below the 128 TiB
//      user address-space ceiling and well above any typical heap.
//
//   2. Each anonymous mmap call:
//        a. Rounds `len` up to the next PAGE_SIZE (4096) boundary.
//        b. Records the pre-advance value of `mmap_bump` as the returned
//           base address.
//        c. Advances `mmap_bump` by the rounded length.
//
//   3. Two consecutive mmaps are guaranteed non-overlapping by
//      construction (the bump never retreats between mmaps).
//
//   4. The allocated addresses are page-aligned (multiple of 4096) because
//      `MMAP_BASE` is page-aligned and we advance in PAGE_SIZE multiples.
//
// This is the same "bookkeeping without hardware" approach that
// `brk::handle` uses for `Process::heap_break` — the kernel tracks the
// reservation; the actual page-frame install is UEFI-only and stubbed
// here via `map_mmap_pages` (a no-op on host test builds).
//
// munmap strategy
// ---------------
// In tier-1 there is no per-mapping free list — the bump pointer never
// retreats. `munmap` returns 0 (success) as a documented no-op, mirroring
// how `brk::handle` treats a shrink on the same foundation. A real per-
// mapping tracker (a `Vec<(base, len)>` tombstone list, or an interval
// tree) is the recommended child task for a future #497 breakout.
//
// Scope guard outcome
// -------------------
// The anonymous bump path is self-contained and does NOT require new
// page-table machinery. The file-backed path is intentionally out of
// scope (returns -ENODEV). Orchestrator can close #497 as partially done:
// the anonymous malloc-backing slice is complete; the recommended child
// tasks are:
//   • #497-a  mmap file-backed: MAP_ANONYMOUS clear, fd valid — needs VFS.
//   • #497-b  munmap frame tracking: per-mapping tombstone list + real free.
//   • #497-c  mmap page-table install: physical frame alloc + PTE install
//             for UEFI target (depends on #527 frame allocator landing).
//
// errno values used
// -----------------
//   ENODEV  = 19  — "No such device"; returned for file-backed mmap requests
//                   where no device/VFS exists to source the pages from.
//   EINVAL  = 22  — "Invalid argument"; returned when len == 0 (mmap(2) man
//                   page: "len must be greater than 0").
//   ENOMEM  = 12  — "Out of memory"; returned if the bump pointer would
//                   overflow the guard ceiling.

use crate::process::current_process_mut;

/// Linux x86_64 syscall number for `mmap(addr, len, prot, flags, fd, off)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_mmap` (= 9).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_mmap`.
pub const SYS_MMAP: u64 = 9;

/// Linux x86_64 syscall number for `munmap(addr, len)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_munmap` (= 11).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_munmap`.
pub const SYS_MUNMAP: u64 = 11;

/// `MAP_ANONYMOUS` flag bit — mapping is not backed by any file; `fd`
/// is ignored and the bytes are initialised to zero (per mmap(2)).
/// Value: 0x20. Source: `linux/include/uapi/asm-generic/mman-common.h`
/// (`MAP_ANONYMOUS = 0x20`); confirmed by musl's
/// `vendor/musl/arch/x86_64/bits/mman.h`.
pub const MAP_ANONYMOUS: u64 = 0x20;

/// System page size (4096 bytes). All mmap allocations are rounded up to
/// this boundary. Matches `AddressSpace::PAGE_SIZE` and the Linux
/// `AT_PAGESZ` auxv constant emitted in `process::process`.
pub const PAGE_SIZE: u64 = 4096;

/// Canonical base of the mmap region on Linux x86_64. Anonymous mmaps
/// are bump-allocated upward from this address.
///
/// 0x7000_0000_0000 = 120 TiB — sits in the [64 TiB, 128 TiB) window
/// that Linux's `TASK_UNMAPPED_BASE` / `vm_unmapped_area` heuristic
/// targets for dynamic libraries and anonymous mappings. Chosen to be
/// well above any ELF load address (typically ~0x40_1000) and any heap
/// (brk-managed, starting just above the BSS), so bump regions from
/// both allocators are naturally non-overlapping.
pub const MMAP_BASE: u64 = 0x0000_7000_0000_0000;

/// Upper bound for the mmap bump region. Any allocation that would advance
/// `mmap_bump` past this address returns `-ENOMEM`. Set to `i64::MAX`
/// (matching `BRK_MAX_ADDR`) — the Linux kernel's practical upper limit
/// for any user virtual address (no address that overflows a signed 64-bit
/// comparison).
pub const MMAP_CEIL: u64 = i64::MAX as u64;

/// Linux errno for "No such device" (19). Returned by `handle_mmap` when
/// the caller requests a file-backed mapping (MAP_ANONYMOUS clear) — no
/// VFS / block device exists in tier-1 to source page contents.
pub const ENODEV: i64 = 19;

/// Linux errno for "Out of memory" (12). Returned by `handle_mmap` when
/// the bump pointer would overflow `MMAP_CEIL`.
pub const ENOMEM: i64 = 12;

/// Linux errno for "Invalid argument" (22). Returned by `handle_mmap`
/// when `len == 0` (mmap(2) requires len > 0).
pub const EINVAL: i64 = 22;

/// Install page-table mappings for a newly-allocated anonymous mmap region
/// on the real x86_64-UEFI target. On host/non-UEFI builds this is a
/// no-op — the bookkeeping bump is the only necessary operation for unit
/// tests. Mirrors `brk::map_heap_pages`.
///
/// SAFETY (future UEFI arm): caller has already validated `base` is
/// page-aligned and `[base, base+len)` lies within `[MMAP_BASE, MMAP_CEIL)`.
/// CPL 0 invariant holds throughout.
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
fn map_mmap_pages(_base: u64, _len: u64) {
    // Boot-integration TODO (#497-c): walk [base, base+len) in PAGE_SIZE
    // steps, allocate physical frames from the frame allocator, and install
    // PTE mappings in the process CR3. Stubbed so the bookkeeping lands
    // independently and is verifiable on host.
}

/// No-op shim for host unit-test builds. The bump bookkeeping tests only
/// need `Process::mmap_bump` to advance; no real page allocation is needed.
#[cfg(not(all(target_os = "uefi", target_arch = "x86_64")))]
#[allow(dead_code)]
fn map_mmap_pages(_base: u64, _len: u64) {
    // No page tables on host — storage-only path for unit tests.
}

/// Handle an `mmap(addr, len, prot, flags, fd, off)` syscall.
///
/// # Anonymous mapping (flags & MAP_ANONYMOUS != 0)
///
/// 1. Validate: returns `-EINVAL` if `len == 0`.
/// 2. Round `len` up to the next PAGE_SIZE (4096) boundary.
/// 3. Check overflow: returns `-ENOMEM` if `mmap_bump + rounded_len`
///    would exceed `MMAP_CEIL`.
/// 4. Record `base = mmap_bump`, advance `mmap_bump += rounded_len`.
/// 5. Call the (no-op on host) `map_mmap_pages(base, rounded_len)` shim.
/// 6. Return `base as i64` — a non-negative page-aligned address.
///
/// # File-backed mapping (flags & MAP_ANONYMOUS == 0)
///
/// Returns `-ENODEV` (-19). File mapping is out of scope for tier-1 (#497).
///
/// # No current process
///
/// Returns `-ENOMEM` as a safe sentinel — consistent with Linux's
/// behaviour when a process has no address space to extend.
///
/// # Arguments
///
/// * `_addr` — requested base address (ignored in tier-1 — bump picks).
/// * `len`   — mapping length in bytes.
/// * `_prot` — protection flags (accepted, ignored in tier-1 — no PTEs).
/// * `flags` — mapping flags; `MAP_ANONYMOUS` (0x20) must be set.
/// * `_fd`   — file descriptor (ignored for anonymous mappings).
/// * `_off`  — file offset (ignored for anonymous mappings).
pub fn handle_mmap(_addr: u64, len: u64, _prot: u64, flags: u64, _fd: u64, _off: u64) -> i64 {
    // File-backed mapping — not supported in tier-1.
    if flags & MAP_ANONYMOUS == 0 {
        return -ENODEV;
    }

    // mmap(2): "length must be greater than 0".
    if len == 0 {
        return -EINVAL;
    }

    current_process_mut(|maybe_proc| {
        let proc = match maybe_proc {
            Some(p) => p,
            // No current process — return -ENOMEM (no address space to map into).
            None => return -ENOMEM,
        };

        // Round len up to the next page boundary.
        let rounded = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

        // Guard against bump overflow.
        if proc.mmap_bump > MMAP_CEIL - rounded {
            return -ENOMEM;
        }

        let base = proc.mmap_bump;
        proc.mmap_bump += rounded;

        // map_mmap_pages is a no-op in tests; on the UEFI target it will
        // walk [base, base+rounded) and install PTEs (future #497-c).
        map_mmap_pages(base, rounded);

        base as i64
    })
}

/// Handle a `munmap(addr, len)` syscall.
///
/// In tier-1 there is no per-mapping free list — the bump allocator
/// never retreats. This handler returns 0 (success) as a documented
/// no-op, matching how `brk::handle` treats a shrink: bookkeeping says
/// "the caller no longer wants this region" but the underlying pages
/// stay mapped (or conceptually reserved) until the process exits.
///
/// A future child task (#497-b) will add a per-mapping tombstone list
/// (`Vec<(base, len)>`) so that munmap'd ranges can be recycled by
/// subsequent mmaps — the interface here is correct; only the free-side
/// tracking is deferred.
///
/// Returns 0 (success) unconditionally. Linux munmap(2) returns 0 on
/// success or -EINVAL for unaligned addresses; tier-1 accepts any addr/len
/// without validation, consistent with the "no page tables" foundation.
pub fn handle_munmap(_addr: u64, _len: u64) -> i64 {
    // Documented no-op: no per-mapping free list in tier-1.
    // Returns 0 (success) — callers (musl free, jemalloc, etc.) check
    // the return value and treat 0 as "the kernel accepted the unmap";
    // their internal structures already track what memory they own.
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::current_process_install;
    use crate::process::current_process_uninstall;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::Process;

    // Convenience: build a minimal Process with a fresh address space.
    fn make_process(pid: u32) -> Process {
        let address_space = AddressSpace::new(0x40_1000);
        Process::new(pid, address_space)
    }

    // -------------------------------------------------------------------
    // Constant value tests
    // -------------------------------------------------------------------

    /// SYS_MMAP is 9 per `linux/arch/x86/include/uapi/asm/unistd_64.h`.
    #[test]
    fn sys_mmap_constant_is_9() {
        assert_eq!(SYS_MMAP, 9, "SYS_MMAP must be 9 per Linux x86_64 unistd_64.h");
    }

    /// SYS_MUNMAP is 11 per `linux/arch/x86/include/uapi/asm/unistd_64.h`.
    #[test]
    fn sys_munmap_constant_is_11() {
        assert_eq!(SYS_MUNMAP, 11, "SYS_MUNMAP must be 11 per Linux x86_64 unistd_64.h");
    }

    /// MAP_ANONYMOUS is 0x20 per `<asm-generic/mman-common.h>`.
    #[test]
    fn map_anonymous_constant_is_0x20() {
        assert_eq!(MAP_ANONYMOUS, 0x20);
    }

    /// PAGE_SIZE is 4096 — the x86_64 base page size.
    #[test]
    fn page_size_constant_is_4096() {
        assert_eq!(PAGE_SIZE, 4096);
    }

    /// MMAP_BASE is page-aligned (multiple of PAGE_SIZE).
    #[test]
    fn mmap_base_is_page_aligned() {
        assert_eq!(MMAP_BASE % PAGE_SIZE, 0, "MMAP_BASE must be page-aligned");
    }

    /// ENODEV is 19 per `<asm-generic/errno-base.h>`.
    #[test]
    fn enodev_constant_is_19() {
        assert_eq!(ENODEV, 19);
    }

    /// EINVAL is 22 per `<asm-generic/errno-base.h>`.
    #[test]
    fn einval_constant_is_22() {
        assert_eq!(EINVAL, 22);
    }

    /// ENOMEM is 12 per `<asm-generic/errno-base.h>`.
    #[test]
    fn enomem_constant_is_12() {
        assert_eq!(ENOMEM, 12);
    }

    // -------------------------------------------------------------------
    // handle_mmap: anonymous path
    // -------------------------------------------------------------------

    /// Anonymous mmap returns a non-negative, page-aligned address.
    #[test]
    fn mmap_anonymous_returns_page_aligned_address() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_install(make_process(1));

        let result = handle_mmap(0, 4096, 0, MAP_ANONYMOUS, u64::MAX, 0);
        current_process_uninstall();

        assert!(result >= 0, "mmap must return a non-negative address on success");
        assert_eq!(result as u64 % PAGE_SIZE, 0, "mmap must return a page-aligned address");
    }

    /// Anonymous mmap returns a non-error address; result must be
    /// exactly MMAP_BASE for the first allocation on a fresh process.
    #[test]
    fn mmap_anonymous_first_allocation_returns_mmap_base() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_install(make_process(2));

        let result = handle_mmap(0, 4096, 0, MAP_ANONYMOUS, u64::MAX, 0);
        current_process_uninstall();

        assert_eq!(
            result as u64, MMAP_BASE,
            "first anonymous mmap on a fresh process must return MMAP_BASE"
        );
    }

    /// Two successive anonymous mmaps return non-overlapping regions.
    /// The second base must be at least `len1` bytes above the first base.
    #[test]
    fn mmap_two_successive_calls_return_non_overlapping_regions() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_install(make_process(3));

        let len1: u64 = 4096;
        let len2: u64 = 8192;
        let base1 = handle_mmap(0, len1, 0, MAP_ANONYMOUS, u64::MAX, 0) as u64;
        let base2 = handle_mmap(0, len2, 0, MAP_ANONYMOUS, u64::MAX, 0) as u64;
        current_process_uninstall();

        assert!(base1 > 0, "first mmap must succeed");
        assert!(base2 > 0, "second mmap must succeed");
        // Non-overlapping: [base1, base1+len1) and [base2, base2+len2) are disjoint.
        // Since the bump is monotonic, base2 >= base1 + len1.
        assert!(
            base2 >= base1 + len1,
            "second mmap base ({:#x}) must be at or above first base+len ({:#x})",
            base2,
            base1 + len1
        );
    }

    /// Successive mmap addresses advance by exactly len-rounded-to-page.
    #[test]
    fn mmap_advances_bump_by_rounded_len() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_install(make_process(4));

        // len = 1 → rounds up to PAGE_SIZE (4096).
        let base1 = handle_mmap(0, 1, 0, MAP_ANONYMOUS, u64::MAX, 0) as u64;
        let base2 = handle_mmap(0, 1, 0, MAP_ANONYMOUS, u64::MAX, 0) as u64;
        current_process_uninstall();

        assert_eq!(
            base2 - base1,
            PAGE_SIZE,
            "mmap(len=1) must advance by exactly PAGE_SIZE (len rounds up to page boundary)"
        );
    }

    /// mmap with len=0 returns -EINVAL.
    #[test]
    fn mmap_zero_len_returns_einval() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_install(make_process(5));

        let result = handle_mmap(0, 0, 0, MAP_ANONYMOUS, u64::MAX, 0);
        current_process_uninstall();

        assert_eq!(result, -EINVAL, "mmap with len=0 must return -EINVAL");
    }

    // -------------------------------------------------------------------
    // handle_mmap: file-backed path
    // -------------------------------------------------------------------

    /// File-backed mmap (MAP_ANONYMOUS clear) returns -ENODEV (-19).
    #[test]
    fn mmap_file_backed_returns_enodev() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_install(make_process(6));

        // flags = MAP_PRIVATE (0x02) — MAP_ANONYMOUS bit NOT set.
        let result = handle_mmap(0, 4096, 0, 0x02, 3, 0);
        current_process_uninstall();

        assert_eq!(result, -ENODEV, "file-backed mmap must return -ENODEV");
    }

    /// File-backed mmap check runs before any process-state check —
    /// even with no process installed, MAP_ANONYMOUS=0 returns -ENODEV.
    #[test]
    fn mmap_file_backed_returns_enodev_even_without_process() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();

        let result = handle_mmap(0, 4096, 0, 0x02, 3, 0);
        assert_eq!(result, -ENODEV);
    }

    // -------------------------------------------------------------------
    // handle_mmap: no-process path
    // -------------------------------------------------------------------

    /// Anonymous mmap with no current process returns -ENOMEM.
    #[test]
    fn mmap_no_process_returns_enomem() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();

        let result = handle_mmap(0, 4096, 0, MAP_ANONYMOUS, u64::MAX, 0);
        assert_eq!(result, -ENOMEM, "mmap with no current process must return -ENOMEM");
    }

    // -------------------------------------------------------------------
    // handle_munmap tests
    // -------------------------------------------------------------------

    /// munmap always returns 0 (success) — documented no-op in tier-1.
    #[test]
    fn munmap_returns_zero() {
        assert_eq!(handle_munmap(0, 0), 0, "munmap must return 0 (success)");
        assert_eq!(handle_munmap(MMAP_BASE, 4096), 0);
        assert_eq!(handle_munmap(0xDEAD_BEEF, 65536), 0);
    }

    /// munmap with no process installed still returns 0 — the no-op
    /// contract holds regardless of process state.
    #[test]
    fn munmap_returns_zero_without_process() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();
        assert_eq!(handle_munmap(MMAP_BASE, 4096), 0);
    }

    // -------------------------------------------------------------------
    // Dispatch wiring tests
    // -------------------------------------------------------------------

    /// `dispatch(SYS_MMAP, ...)` with MAP_ANONYMOUS returns a non-
    /// negative page-aligned address. Verifies the dispatcher routes
    /// rax=9 to mmap::handle_mmap with the right argument order.
    #[test]
    fn dispatch_sys_mmap_anonymous_routes_correctly() {
        use crate::syscall::dispatch::{dispatch, SYS_MMAP};
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_install(make_process(10));

        // dispatch(rax, rdi, rsi, rdx, r10, r8, r9)
        // mmap:     (rax, addr, len, prot, flags, fd, off)
        let result = dispatch(SYS_MMAP, 0, 4096, 0, MAP_ANONYMOUS, u64::MAX, 0);
        current_process_uninstall();

        assert!(result >= 0, "dispatch(SYS_MMAP, anonymous) must return non-negative");
        assert_eq!(result as u64 % PAGE_SIZE, 0, "dispatch(SYS_MMAP) must return page-aligned address");
    }

    /// `dispatch(SYS_MUNMAP, ...)` returns 0. Verifies dispatch routes
    /// rax=11 to mmap::handle_munmap.
    #[test]
    fn dispatch_sys_munmap_routes_correctly() {
        use crate::syscall::dispatch::{dispatch, SYS_MUNMAP};
        assert_eq!(
            dispatch(SYS_MUNMAP, MMAP_BASE, 4096, 0, 0, 0, 0),
            0,
            "dispatch(SYS_MUNMAP) must return 0"
        );
    }

    /// `dispatch(SYS_MMAP, ..., flags=0x02, ...)` (file-backed) returns
    /// -ENODEV via the dispatch table.
    #[test]
    fn dispatch_sys_mmap_file_backed_returns_enodev() {
        use crate::syscall::dispatch::{dispatch, SYS_MMAP};
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();

        let result = dispatch(SYS_MMAP, 0, 4096, 0, 0x02, 3, 0);
        assert_eq!(result, -ENODEV);
    }

    // -------------------------------------------------------------------
    // Process::mmap_bump initialisation test
    // -------------------------------------------------------------------

    /// `Process::new` initialises mmap_bump to MMAP_BASE.
    #[test]
    fn process_new_initialises_mmap_bump_to_mmap_base() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(99, address_space);
        assert_eq!(
            proc.mmap_bump,
            MMAP_BASE,
            "mmap_bump must be initialised to MMAP_BASE after Process::new"
        );
    }
}
