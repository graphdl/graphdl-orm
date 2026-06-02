// crates/arest-kernel/src/syscall/rt_sigprocmask.rs
//
// Linux x86_64 syscall 14:
// `rt_sigprocmask(int how, const sigset_t *set, sigset_t *oldset,
//                 size_t sigsetsize)`.
// Per #548 (the signal-handling plumbing). Blocks / unblocks / sets the
// thread's signal mask, returning the previous mask in `oldset`. Tier-1
// is single-threaded so the "thread" mask lives on the Process's
// `SignalState` (`process::signal`).
//
// Linux x86_64 number: `__NR_rt_sigprocmask = 14`
// (`vendor/musl/arch/x86_64/bits/syscall.h.in:15`).
//
// The three `how` operations
// ---------------------------
//   SIG_BLOCK   (0) — OR the bits in `set` INTO the current mask
//   SIG_UNBLOCK (1) — clear the bits in `set` FROM the current mask
//   SIG_SETMASK (2) — replace the mask wholesale with `set`
// musl's `pthread_sigmask` (`vendor/musl/src/thread/pthread_sigmask.c`)
// validates `how` (returns EINVAL when `set` is non-null and
// `how > SIG_SETMASK`) before the syscall; we do the same kernel-side
// so a raw syscall (bypassing libc) is rejected too.
//
// sigsetsize
// ----------
// The fourth argument must equal `_NSIG/8 = 8` (the 64-bit kernel
// sigset width); the kernel rejects any other value with -EINVAL. musl
// always passes exactly 8.
//
// The sigset pointer model
// ------------------------
// `set` and `oldset` are pointers to a 64-bit sigset (8 bytes — one bit
// per signal, bit `signum-1`). We `core::ptr::read` `set` as a `u64`
// and `core::ptr::write` the old mask to `oldset`, under the tier-1
// identity mapping (same as `ioctl` / `arch_prctl` / `rt_sigaction`:
// null → -EFAULT, non-null treated as a valid kernel pointer; real page
// tables + copy_to_user land with #527 / #561).
//
// SIGKILL / SIGSTOP are un-blockable — the `SignalState::update_mask`
// primitive forces their bits clear out of any resulting mask, matching
// the Linux kernel.
//
// Null-pointer semantics: a null `set` is the query form (report
// `oldset`, change nothing); a null `oldset` updates without reporting.
// Both are valid Linux behaviour, not faults — same rationale as
// `rt_sigaction` (no -EFAULT for null pointers; real `copy_from_user`
// validation lands with #561). The errno surface is EINVAL + ESRCH.
//
// errno values
//   EINVAL = 22 — bad `how` (with non-null `set`), or a sigsetsize != 8
//   ESRCH  = 3  — no current process installed (see rt_sigaction)

use crate::process::current_process_signals;
use crate::process::signal::{KERNEL_SIGSET_SIZE, SIG_SETMASK};
use crate::syscall::dispatch::EINVAL;
use crate::syscall::rt_sigaction::ESRCH;

/// Handle an `rt_sigprocmask(how, set, oldset, sigsetsize)` syscall.
///
/// * Validates `sigsetsize == 8` → -EINVAL.
/// * Validates `how` is SIG_BLOCK/UNBLOCK/SETMASK when `set` is
///   non-null → -EINVAL otherwise.
/// * If `oldset` is non-null, writes the CURRENT mask there (before the
///   update).
/// * If `set` is non-null, reads the 64-bit sigset from it and applies
///   the `how` operation; SIGKILL/SIGSTOP bits are forced clear.
/// * Returns 0 on success.
///
/// `set == 0` is the query form: writes `oldset`, changes nothing (and
/// skips the `how` validation, matching Linux — a null `set` makes
/// `how` irrelevant). `oldset == 0` updates without reporting.
pub fn handle(how: i32, set: u64, oldset: u64, sigsetsize: u64) -> i64 {
    if sigsetsize as usize != KERNEL_SIGSET_SIZE {
        return -EINVAL;
    }
    // Validate `how` only when `set` is supplied — Linux/musl ignore
    // `how` entirely for the query form (`set == 0`).
    if set != 0 && (how < 0 || how > SIG_SETMASK) {
        return -EINVAL;
    }

    current_process_signals(|maybe_sig| {
        let sig = match maybe_sig {
            Some(s) => s,
            None => return -ESRCH,
        };

        // Report the old mask first (Linux writes oldset even for the
        // query form).
        let old = sig.blocked_mask();
        if oldset != 0 {
            // SAFETY: oldset is non-null; under the tier-1 identity
            // mapping it is a valid kernel-space pointer the caller owns
            // (enforced in tests via a stack buffer). Writing 8 bytes is
            // in-bounds for the caller's sigset_t.
            unsafe { core::ptr::write(oldset as *mut u64, old) };
        }

        // Apply the update if `set` was supplied.
        if set != 0 {
            // SAFETY: set is non-null; same identity-mapping rationale.
            // Reading 8 bytes is in-bounds for the caller's sigset_t.
            let bits = unsafe { core::ptr::read(set as *const u64) };
            // `how` was validated above; update_mask returns None only
            // for an unrecognised how, which can't happen here.
            if sig.update_mask(how, bits).is_none() {
                return -EINVAL;
            }
        }

        0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::signal::{SIGKILL, SIGSTOP, SIG_BLOCK, SIG_UNBLOCK};
    use crate::process::{current_process_install, current_process_uninstall, Process};

    fn with_process<F: FnOnce()>(body: F) {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        current_process_install(Process::new(7, address_space));
        body();
        current_process_uninstall();
    }

    /// SIG_BLOCK ORs the supplied bits into the mask and returns the old
    /// (empty) mask in `oldset`.
    #[test]
    fn block_ors_bits_and_returns_old() {
        with_process(|| {
            // Block SIGINT (2) → bit 1.
            let set: u64 = 1 << 1;
            let mut old: u64 = 0xffff_ffff_ffff_ffff;
            let r = handle(
                SIG_BLOCK,
                &set as *const u64 as u64,
                &mut old as *mut u64 as u64,
                KERNEL_SIGSET_SIZE as u64,
            );
            assert_eq!(r, 0);
            assert_eq!(old, 0, "old mask was empty");
            current_process_signals(|s| {
                let s = s.unwrap();
                assert!(s.is_blocked(2));
                assert_eq!(s.blocked_mask(), 1 << 1);
            });
        });
    }

    /// SIG_UNBLOCK clears the supplied bits; the old mask reflects the
    /// pre-unblock state.
    #[test]
    fn unblock_clears_bits() {
        with_process(|| {
            // First block SIGINT (2) + SIGTERM (15).
            let block: u64 = (1 << 1) | (1 << 14);
            assert_eq!(
                handle(SIG_BLOCK, &block as *const u64 as u64, 0, KERNEL_SIGSET_SIZE as u64),
                0
            );
            // Now unblock SIGINT.
            let unblock: u64 = 1 << 1;
            let mut old: u64 = 0;
            assert_eq!(
                handle(
                    SIG_UNBLOCK,
                    &unblock as *const u64 as u64,
                    &mut old as *mut u64 as u64,
                    KERNEL_SIGSET_SIZE as u64
                ),
                0
            );
            assert_eq!(old, (1 << 1) | (1 << 14), "old mask had both blocked");
            current_process_signals(|s| {
                let s = s.unwrap();
                assert!(!s.is_blocked(2), "SIGINT unblocked");
                assert!(s.is_blocked(15), "SIGTERM still blocked");
            });
        });
    }

    /// SIG_SETMASK replaces the mask wholesale.
    #[test]
    fn setmask_replaces_mask() {
        with_process(|| {
            // Block bit 5 first.
            let pre: u64 = 1 << 5;
            assert_eq!(
                handle(SIG_BLOCK, &pre as *const u64 as u64, 0, KERNEL_SIGSET_SIZE as u64),
                0
            );
            // SETMASK to bit 1 only.
            let set: u64 = 1 << 1;
            let mut old: u64 = 0;
            assert_eq!(
                handle(
                    SIG_SETMASK,
                    &set as *const u64 as u64,
                    &mut old as *mut u64 as u64,
                    KERNEL_SIGSET_SIZE as u64
                ),
                0
            );
            assert_eq!(old, 1 << 5);
            current_process_signals(|s| {
                let s = s.unwrap();
                assert!(!s.is_blocked(6), "bit 5 cleared by SETMASK");
                assert!(s.is_blocked(2), "bit 1 set by SETMASK");
            });
        });
    }

    /// The query form (`set == 0`) reports the current mask in `oldset`
    /// and changes nothing — and skips `how` validation (Linux ignores
    /// `how` when `set` is null).
    #[test]
    fn null_set_queries_current_mask() {
        with_process(|| {
            let block: u64 = 1 << 3;
            assert_eq!(
                handle(SIG_BLOCK, &block as *const u64 as u64, 0, KERNEL_SIGSET_SIZE as u64),
                0
            );
            // Query with a deliberately-bogus `how` (99) — must still
            // succeed because `set` is null.
            let mut old: u64 = 0;
            assert_eq!(handle(99, 0, &mut old as *mut u64 as u64, KERNEL_SIGSET_SIZE as u64), 0);
            assert_eq!(old, 1 << 3);
            // Mask unchanged.
            current_process_signals(|s| assert_eq!(s.unwrap().blocked_mask(), 1 << 3));
        });
    }

    /// A bad `how` (with a non-null `set`) returns -EINVAL.
    #[test]
    fn bad_how_with_set_returns_einval() {
        with_process(|| {
            let set: u64 = 1 << 1;
            let p = &set as *const u64 as u64;
            assert_eq!(handle(3, p, 0, KERNEL_SIGSET_SIZE as u64), -EINVAL);
            assert_eq!(handle(-1, p, 0, KERNEL_SIGSET_SIZE as u64), -EINVAL);
            assert_eq!(handle(99, p, 0, KERNEL_SIGSET_SIZE as u64), -EINVAL);
        });
    }

    /// A wrong `sigsetsize` returns -EINVAL.
    #[test]
    fn wrong_sigsetsize_returns_einval() {
        with_process(|| {
            let set: u64 = 0;
            let p = &set as *const u64 as u64;
            assert_eq!(handle(SIG_BLOCK, p, 0, 16), -EINVAL);
            assert_eq!(handle(SIG_BLOCK, p, 0, 4), -EINVAL);
        });
    }

    /// SIGKILL / SIGSTOP can't be blocked even via SIG_SETMASK — the
    /// resulting mask has their bits forced clear. The mask is
    /// RESPECTED: a subsequent `is_blocked` check on a normal signal
    /// reflects what SETMASK installed, but the two un-blockables stay
    /// deliverable.
    #[test]
    fn sigkill_sigstop_stay_unblocked() {
        with_process(|| {
            // SETMASK to block everything.
            let all: u64 = u64::MAX;
            let mut old: u64 = 0;
            assert_eq!(
                handle(
                    SIG_SETMASK,
                    &all as *const u64 as u64,
                    &mut old as *mut u64 as u64,
                    KERNEL_SIGSET_SIZE as u64
                ),
                0
            );
            current_process_signals(|s| {
                let s = s.unwrap();
                assert!(!s.is_blocked(SIGKILL), "SIGKILL must stay deliverable");
                assert!(!s.is_blocked(SIGSTOP), "SIGSTOP must stay deliverable");
                // A normal signal IS blocked — the mask is respected.
                assert!(s.is_blocked(2));
                assert!(s.is_blocked(64));
            });
        });
    }

    /// With no current process installed, `rt_sigprocmask` returns
    /// -ESRCH (after the cheap size/how checks pass).
    #[test]
    fn no_process_returns_esrch() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();
        let set: u64 = 0;
        let r = handle(SIG_BLOCK, &set as *const u64 as u64, 0, KERNEL_SIGSET_SIZE as u64);
        assert_eq!(r, -ESRCH);
    }

    /// Dispatch wiring: `dispatch(SYS_RT_SIGPROCMASK, ...)` routes here.
    #[test]
    fn dispatch_routes_rt_sigprocmask() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        use crate::syscall::dispatch::{dispatch, SYS_RT_SIGPROCMASK};
        let address_space = AddressSpace::new(0x40_1000);
        current_process_install(Process::new(7, address_space));
        let set: u64 = 1 << 7;
        let mut old: u64 = 0;
        let r = dispatch(
            SYS_RT_SIGPROCMASK,
            SIG_BLOCK as u64,
            &set as *const u64 as u64,
            &mut old as *mut u64 as u64,
            KERNEL_SIGSET_SIZE as u64,
            0,
            0,
        );
        assert_eq!(r, 0);
        assert_eq!(old, 0);
        current_process_signals(|s| assert!(s.unwrap().is_blocked(8)));
        current_process_uninstall();
    }
}
