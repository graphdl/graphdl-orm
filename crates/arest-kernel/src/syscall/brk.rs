// crates/arest-kernel/src/syscall/brk.rs
//
// Linux x86_64 syscall 12: `brk(unsigned long addr)`. Per #509
// (heap-break / sbrk track). SYS_BRK is the only kernel-side heap
// syscall — the C library's `sbrk(3)` is implemented in userspace by
// calling `brk(2)` twice (once to query, once to advance); no separate
// SYS_SBRK exists on Linux x86_64 (confirmed from
// `linux/arch/x86/include/uapi/asm/unistd_64.h`: there is no
// `__NR_sbrk` entry in the x86_64 table).
//
// Linux x86_64 number: `__NR_brk = 12`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`).
//
// Linux brk(2) semantics (never fails — returns the resulting break)
// -------------------------------------------------------------------
// Unlike most syscalls, `brk` NEVER returns a negative errno. It
// always returns the current (or new) program-break value:
//
//   brk(0)         → return current heap_break (query the break).
//   brk(addr ≥ current_break or ≥ heap_start)
//                  → set heap_break = addr, return addr.
//   brk(addr too low / invalid)
//                  → leave heap_break unchanged, return heap_break.
//
// The "never errors" contract is stated in the Linux `brk(2)` man
// page: "On success, brk() returns zero. On error, brk() returns -1
// and sets errno to ENOMEM." — BUT that describes the C library
// wrapper (`man 2 brk`), not the raw syscall. At the raw syscall
// level (the value returned in rax after SYSCALL) the kernel just
// returns the resulting break regardless; the libc wrapper then
// inspects whether rax changed as expected and synthesises ENOMEM if
// not. We match the kernel convention: always return the resulting
// break as a non-negative i64.
//
// "Too low" definition for tier-1
// --------------------------------
// We define "too low" as: `addr < HEAP_FLOOR`. The floor is 1 (any
// non-zero address is at or above it) because tier-1 has no real page
// table to anchor a specific heap start VMA. Practically, addr = 0 is
// the "query" form (handled before this check), and every real musl /
// glibc brk call will supply a reasonable address well above 0.
// HEAP_FLOOR is exported as a constant so unit tests can anchor their
// assertions and future boot-integration can override it once the UEFI
// linker script pins the heap start VMA.
//
// Maximum break
// -------------
// BRK_MAX_ADDR is set to `u64::MAX / 2` (i.e., `i64::MAX as u64`),
// matching the Linux kernel's practical limit ("no address that would
// overflow a signed comparison with the stack top"). Any addr above
// BRK_MAX_ADDR is treated as invalid and the current break is
// returned unchanged.
//
// Implementation split (mirrors arch_prctl pattern)
// --------------------------------------------------
// The handler has two concerns:
//
//   1. **Kernel-side bookkeeping**: update / query `Process::heap_break`.
//      Pure Rust — testable on any host.
//
//   2. **Page-table mapping**: on the real x86_64-UEFI target, the
//      kernel must map new heap pages when the break advances. This is
//      gated behind `#[cfg(all(target_os = "uefi", target_arch =
//      "x86_64"))]` — a no-op shim (`map_heap_pages`) stands in for
//      host tests and non-UEFI targets. Boot integration (#527 /
//      follow-up slice) fills in the real mapping; the bookkeeping is
//      self-consistent without it.

use crate::process::current_process_mut;

/// Linux x86_64 syscall number for `brk(addr)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_brk` (= 12).
/// Declared here (mirrors the dispatch.rs constant) so
/// `brk::tests` can assert the number without importing dispatch.
pub const SYS_BRK: u64 = 12;

/// Minimum valid heap address. Any non-zero `addr` at or above this
/// floor is accepted as a valid break target. Tier-1 sets this to 1
/// (no real heap-start VMA yet); the boot-integration slice will
/// anchor this to the ELF heap start once the linker script is set.
/// Exported so tests can anchor boundary assertions.
pub const HEAP_FLOOR: u64 = 1;

/// Maximum valid heap address. Mirrors the Linux kernel's practical
/// upper limit: `i64::MAX` (no address that overflows a signed 64-bit
/// comparison). Any `addr` above this is treated as invalid.
pub const BRK_MAX_ADDR: u64 = i64::MAX as u64;

/// Map newly-allocated heap pages on the real x86_64-UEFI target.
/// This is the page-table-install half of the brk syscall — it maps
/// `[old_break, new_break)` into the process address space. On the
/// real target this calls into the frame allocator + page-table
/// walker (pending boot-integration slice); on host unit-test builds
/// and non-x86_64-UEFI targets it is a no-op so the bookkeeping tests
/// compile and run without hardware.
///
/// Arguments
/// ---------
/// * `_old_break` — the break value BEFORE this advance (the base of
///   the region to map, inclusive).
/// * `_new_break` — the break value AFTER this advance (the first
///   address past the new heap top, exclusive). Always `> old_break`.
///
/// On shrink (`new_break < old_break`) the real implementation would
/// unmap pages; in tier-1 the bookkeeping already records the new
/// (lower) break, so no unmap is needed here — we just update the
/// field. The UEFI arm will add TLB invalidation + frame freeing when
/// it lands.
///
/// SAFETY (future UEFI arm): the caller has already validated that
/// `new_break` is in range (`HEAP_FLOOR ≤ new_break ≤ BRK_MAX_ADDR`).
/// The CPL 0 invariant holds — the kernel is always in ring 0 when
/// the syscall handler fires.
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
fn map_heap_pages(_old_break: u64, _new_break: u64) {
    // Boot-integration TODO (#527 follow-up): walk [old_break,
    // new_break) in PAGE_SIZE steps, allocate physical frames from
    // the frame allocator, and install PTE mappings in the process
    // CR3. For now this is a stub so the bookkeeping can land
    // independently and be verified in unit tests.
}

/// No-op shim for host (Windows / Linux / macOS) unit tests and
/// non-x86_64-UEFI targets. The bookkeeping tests only need the
/// `Process::heap_break` field update, not real page allocation.
#[cfg(not(all(target_os = "uefi", target_arch = "x86_64")))]
#[allow(dead_code)]
fn map_heap_pages(_old_break: u64, _new_break: u64) {
    // No page tables on host — storage-only path for unit tests.
}

/// Handle a `brk(addr)` syscall.
///
/// * `addr == 0`: return current `heap_break` (query semantics).
/// * `HEAP_FLOOR ≤ addr ≤ BRK_MAX_ADDR`: record `heap_break = addr`,
///   call the (stub) `map_heap_pages` shim, return `addr`.
/// * Any other `addr` (too low, or overflow): return current
///   `heap_break` unchanged.
///
/// Follows the Linux raw-syscall convention: always returns the
/// (resulting) break as a non-negative `i64`. Never returns a
/// negative errno. The C library wrapper (`man 3 brk` / `man 2 brk`
/// libc layer) compares the returned value to the requested value and
/// synthesises `ENOMEM` in userspace if they differ.
pub fn handle(addr: u64) -> i64 {
    current_process_mut(|maybe_proc| {
        let proc = match maybe_proc {
            Some(p) => p,
            // No current process installed — return 0 as a safe
            // sentinel (mirrors getpid::handle's None → 0 pattern).
            None => return 0,
        };

        // brk(0) — query current break.
        if addr == 0 {
            return proc.heap_break as i64;
        }

        // Validate the requested address.
        if addr < HEAP_FLOOR || addr > BRK_MAX_ADDR {
            // Invalid address — return current break unchanged.
            return proc.heap_break as i64;
        }

        // Valid address — update bookkeeping, invoke the (stub) page
        // mapper, return the new break.
        let old_break = proc.heap_break;
        proc.heap_break = addr;
        // map_heap_pages is a no-op in tests; on the UEFI target it
        // will walk [old_break, addr) and install PTEs. We call it
        // even on shrink (addr < old_break) — the UEFI arm will add
        // the unmap path there; for now both expand and shrink just
        // update the field.
        map_heap_pages(old_break, addr);
        addr as i64
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::current_process_install;
    use crate::process::current_process_uninstall;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::Process;

    /// `brk(0)` — query form — returns the current heap_break.
    /// After `Process::new`, heap_break is 0, so brk(0) returns 0.
    #[test]
    fn brk_zero_returns_current_break_initially_zero() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(1, address_space);
        current_process_install(proc);

        let result = handle(0);
        current_process_uninstall();

        assert_eq!(result, 0, "brk(0) must return current break (0 at init)");
    }

    /// `brk(valid_addr)` stores the new break in Process::heap_break
    /// and returns the address.
    #[test]
    fn brk_valid_addr_sets_heap_break_and_returns_addr() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(2, address_space);
        current_process_install(proc);

        let new_break: u64 = 0x0000_7fff_0000_0000;
        let result = handle(new_break);
        assert_eq!(result, new_break as i64, "brk(addr) must return addr on success");

        // Verify the field was stored.
        crate::process::current_process_mut(|maybe_proc| {
            let p = maybe_proc.expect("process must be installed");
            assert_eq!(p.heap_break, new_break, "heap_break field must equal new_break");
        });

        current_process_uninstall();
    }

    /// `brk(0)` after a prior valid brk returns the stored break, not 0.
    #[test]
    fn brk_zero_after_set_returns_stored_break() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(3, address_space);
        current_process_install(proc);

        let new_break: u64 = 0x1234_5000;
        let set_result = handle(new_break);
        assert_eq!(set_result, new_break as i64);

        // brk(0) now returns the stored value.
        let query_result = handle(0);
        assert_eq!(
            query_result, new_break as i64,
            "brk(0) must return the previously-set break"
        );

        current_process_uninstall();
    }

    /// Multiple advances: each brk call advances the stored break.
    #[test]
    fn brk_multiple_advances_store_latest_break() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(4, address_space);
        current_process_install(proc);

        let first: u64 = 0x1000_0000;
        let second: u64 = 0x2000_0000;
        let third: u64 = 0x3000_0000;

        assert_eq!(handle(first), first as i64);
        assert_eq!(handle(second), second as i64);
        assert_eq!(handle(third), third as i64);

        crate::process::current_process_mut(|maybe_proc| {
            let p = maybe_proc.expect("process");
            assert_eq!(p.heap_break, third);
        });

        current_process_uninstall();
    }

    /// `brk(addr)` where `addr > BRK_MAX_ADDR` is invalid — returns
    /// current break unchanged.
    #[test]
    fn brk_overflow_addr_returns_current_break_unchanged() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(5, address_space);
        current_process_install(proc);

        // Set a baseline break first.
        let baseline: u64 = 0x8000_0000;
        assert_eq!(handle(baseline), baseline as i64);

        // Address above BRK_MAX_ADDR — treated as invalid.
        let overflow_addr: u64 = BRK_MAX_ADDR + 1;
        let result = handle(overflow_addr);
        assert_eq!(
            result, baseline as i64,
            "brk with addr > BRK_MAX_ADDR must return current break unchanged"
        );

        // heap_break must still be baseline.
        crate::process::current_process_mut(|maybe_proc| {
            let p = maybe_proc.expect("process");
            assert_eq!(p.heap_break, baseline);
        });

        current_process_uninstall();
    }

    /// `brk` with no current process returns 0 (safe sentinel).
    #[test]
    fn brk_no_current_process_returns_zero() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();
        assert_eq!(handle(0), 0);
        assert_eq!(handle(0x1000_0000), 0);
    }

    /// Shrink: `brk(addr < current_break)` is valid — the break is
    /// lowered and the new (lower) value is returned.
    #[test]
    fn brk_shrink_lowers_break_and_returns_new_break() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(6, address_space);
        current_process_install(proc);

        let high: u64 = 0x4000_0000;
        let low: u64 = 0x2000_0000;
        assert_eq!(handle(high), high as i64);

        let result = handle(low);
        assert_eq!(result, low as i64, "brk shrink must return the new lower break");

        crate::process::current_process_mut(|maybe_proc| {
            let p = maybe_proc.expect("process");
            assert_eq!(p.heap_break, low, "heap_break must be lowered on shrink");
        });

        current_process_uninstall();
    }

    /// Constant value checks — SYS_BRK is 12, HEAP_FLOOR is 1,
    /// BRK_MAX_ADDR is i64::MAX.
    #[test]
    fn brk_constants_match_expected_values() {
        assert_eq!(SYS_BRK, 12, "SYS_BRK must be 12 per Linux x86_64 unistd_64.h");
        assert_eq!(HEAP_FLOOR, 1, "HEAP_FLOOR must be 1 for tier-1 (no real heap VMA yet)");
        assert_eq!(
            BRK_MAX_ADDR,
            i64::MAX as u64,
            "BRK_MAX_ADDR must be i64::MAX — mirrors Linux's signed-address upper limit"
        );
    }

    /// `Process::new` initialises heap_break to 0.
    #[test]
    fn process_new_initialises_heap_break_to_zero() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(99, address_space);
        assert_eq!(proc.heap_break, 0, "heap_break must be 0 after Process::new");
    }
}
