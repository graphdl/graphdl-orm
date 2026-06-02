// crates/arest-kernel/src/syscall/rt_sigaction.rs
//
// Linux x86_64 syscall 13:
// `rt_sigaction(int signum, const struct k_sigaction *act,
//               struct k_sigaction *oldact, size_t sigsetsize)`.
// Per #548 (the signal-handling plumbing — the foundation #549/#550/
// #551 build on). Installs / replaces the per-process disposition for
// one signal, returning the previous disposition in `oldact`.
//
// Linux x86_64 number: `__NR_rt_sigaction = 13`
// (`vendor/musl/arch/x86_64/bits/syscall.h.in:14`).
//
// Why "rt_" and the fourth sigsetsize arg
// ---------------------------------------
// The original `sigaction` (no `rt_`) used a 32-bit signal set and
// pre-dates the real-time signals (SIGRTMIN..SIGRTMAX). Modern libc
// (glibc, musl) only ever issues the `rt_` variant with the 64-bit
// set; the kernel validates the set size via the fourth argument
// `sigsetsize`, which must equal `_NSIG/8 = 8` (anything else →
// -EINVAL). musl always passes exactly 8
// (`__syscall(SYS_rt_sigaction, sig, ..., _NSIG/8)` in
// `vendor/musl/src/signal/sigaction.c`).
//
// The kernel struct (k_sigaction), not libc's struct sigaction
// -------------------------------------------------------------
// The raw syscall sees `struct k_sigaction` (32 bytes on x86_64:
// handler / flags / restorer / mask — see `process::signal::SigAction`),
// which musl marshals the libc `struct sigaction` into before the
// syscall. We `core::ptr::read` the `act` pointer as that 32-byte
// struct and `core::ptr::write` the old disposition back to `oldact`,
// under the tier-1 identity mapping (same pointer model as `ioctl` /
// `arch_prctl`: no real page tables yet, #527; null pointer → -EFAULT,
// non-null treated as a valid kernel pointer).
//
// SIGKILL / SIGSTOP are un-catchable
// ----------------------------------
// Linux refuses to change the disposition of SIGKILL (9) or SIGSTOP
// (19) — `rt_sigaction` returns -EINVAL. We reject them up front (after
// the generic range check) so a process can't install a handler that
// would let it survive `kill -9` (#549's SIGKILL path depends on this
// guarantee).
//
// Null-pointer semantics (no EFAULT for null act/oldact)
// -------------------------------------------------------
// A null `act` is the query form (report `oldact`, install nothing); a
// null `oldact` installs without reporting — both are valid Linux
// behaviour, NOT faults. So this handler does not return -EFAULT for
// null `act`/`oldact` (a genuinely-bad non-null pointer would fault on
// deref, but under the tier-1 identity mapping there are no real page
// tables to fault against yet — #527; `copy_from_user` validation lands
// with #561). The errno surface here is therefore EINVAL + ESRCH.
//
// errno values
//   EINVAL = 22 — out-of-range signum, un-catchable signal, or a
//                 sigsetsize != 8
//   ESRCH  = 3  — no current process installed (called before the
//                 ring-3 gate — #552 — installs one)

use crate::process::current_process_signals;
use crate::process::signal::{SigAction, KERNEL_SIGSET_SIZE, SIGKILL, SIGSTOP};
use crate::syscall::dispatch::EINVAL;

/// Linux errno for "No such process". Returned when a signal syscall
/// runs with no current process installed — the closest Linux analogue
/// (a signal syscall with no addressable task). Value 3, from
/// `<asm-generic/errno-base.h>:ESRCH`.
pub const ESRCH: i64 = 3;

/// Handle an `rt_sigaction(signum, act, oldact, sigsetsize)` syscall.
///
/// * Validates `sigsetsize == 8` (the kernel sigset width) → -EINVAL.
/// * Validates `signum` is in 1..=64 and is not SIGKILL/SIGSTOP →
///   -EINVAL otherwise.
/// * If `oldact` is non-null, writes the CURRENT disposition there
///   (before the install) so the caller sees what it replaced.
/// * If `act` is non-null, reads the new `k_sigaction` from it and
///   installs it as the disposition for `signum`.
/// * Returns 0 on success.
///
/// `act` and `oldact` may each independently be null: `act == 0`
/// queries the current disposition (writes `oldact`, installs nothing);
/// `oldact == 0` installs without reporting the old one. Both null is a
/// validated no-op that returns 0.
pub fn handle(signum: i32, act: u64, oldact: u64, sigsetsize: u64) -> i64 {
    // The kernel sigset width is fixed at 8 bytes; reject any other.
    if sigsetsize as usize != KERNEL_SIGSET_SIZE {
        return -EINVAL;
    }
    // Range + un-catchable screen. The pure storage primitive
    // (`SignalState::set_action`) trusts the caller to have screened
    // SIGKILL/SIGSTOP, so we do it here where -EINVAL can be returned.
    if signum < 1 || signum as usize > crate::process::signal::MAX_SIGNAL {
        return -EINVAL;
    }
    if signum == SIGKILL || signum == SIGSTOP {
        return -EINVAL;
    }

    current_process_signals(|maybe_sig| {
        let sig = match maybe_sig {
            Some(s) => s,
            None => return -ESRCH,
        };

        // Snapshot the current disposition BEFORE any install so
        // `oldact` reflects what was replaced (or just the current
        // disposition when `act` is null).
        let old = match sig.action(signum) {
            Some(a) => a,
            // Unreachable — signum was range-checked above — but be
            // defensive rather than panic in the kernel.
            None => return -EINVAL,
        };

        // Report the old disposition first (Linux writes oldact even
        // when act is null — the query form).
        if oldact != 0 {
            // SAFETY: oldact is non-null; under the tier-1 identity
            // mapping it is a valid kernel-space pointer the caller
            // owns (enforced in tests via a stack buffer). Writing a
            // 32-byte k_sigaction is in-bounds for the caller's struct.
            unsafe { core::ptr::write(oldact as *mut SigAction, old) };
        }

        // Install the new disposition if act was supplied.
        if act != 0 {
            // SAFETY: act is non-null; same identity-mapping rationale.
            // Reading a 32-byte k_sigaction is in-bounds for the
            // caller's struct.
            let new = unsafe { core::ptr::read(act as *const SigAction) };
            sig.set_action(signum, new);
        }

        0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::signal::{SIG_DFL, SIG_IGN};
    use crate::process::{current_process_install, current_process_uninstall, Process};

    /// Install a fresh process for the duration of `body`, then tear it
    /// down. Serialised via `CURRENT_PROCESS_TEST_LOCK` (the kernel-wide
    /// process-singleton test lock) so parallel tests don't clobber the
    /// shared `CURRENT_PROCESS` slot.
    fn with_process<F: FnOnce()>(body: F) {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        current_process_install(Process::new(7, address_space));
        body();
        current_process_uninstall();
    }

    /// `rt_sigaction` installs a handler and returns the old (default)
    /// disposition in `oldact`. The headline round-trip.
    #[test]
    fn install_handler_returns_old_default() {
        with_process(|| {
            let new = SigAction {
                handler: 0x40_2000,
                flags: 0,
                restorer: 0x40_3000,
                mask: 0,
            };
            let mut old = core::mem::MaybeUninit::<SigAction>::uninit();
            // SIGINT (2).
            let r = handle(
                2,
                &new as *const SigAction as u64,
                old.as_mut_ptr() as u64,
                KERNEL_SIGSET_SIZE as u64,
            );
            assert_eq!(r, 0);
            // Old disposition was the default (SIG_DFL).
            let old = unsafe { old.assume_init() };
            assert_eq!(old.handler, SIG_DFL);
            assert!(old.is_default());
            // The new disposition is now live in the process.
            current_process_signals(|s| {
                let live = s.unwrap().action(2).unwrap();
                assert_eq!(live, new);
            });
        });
    }

    /// A second `rt_sigaction` returns the PREVIOUSLY-installed handler
    /// in `oldact`, not the default.
    #[test]
    fn second_install_returns_previous_handler() {
        with_process(|| {
            let first = SigAction {
                handler: 0xaaaa,
                flags: 0,
                restorer: 0,
                mask: 0,
            };
            let second = SigAction {
                handler: 0xbbbb,
                flags: 0,
                restorer: 0,
                mask: 0,
            };
            // Install first (discard oldact).
            assert_eq!(
                handle(10, &first as *const SigAction as u64, 0, KERNEL_SIGSET_SIZE as u64),
                0
            );
            // Install second, capture old.
            let mut old = core::mem::MaybeUninit::<SigAction>::uninit();
            assert_eq!(
                handle(
                    10,
                    &second as *const SigAction as u64,
                    old.as_mut_ptr() as u64,
                    KERNEL_SIGSET_SIZE as u64
                ),
                0
            );
            let old = unsafe { old.assume_init() };
            assert_eq!(old, first);
        });
    }

    /// `act == 0` is the query form: writes the current disposition to
    /// `oldact`, installs nothing.
    #[test]
    fn null_act_queries_without_installing() {
        with_process(|| {
            let ign = SigAction {
                handler: SIG_IGN,
                ..SigAction::default_action()
            };
            // Install SIG_IGN for SIGTERM (15).
            assert_eq!(
                handle(15, &ign as *const SigAction as u64, 0, KERNEL_SIGSET_SIZE as u64),
                0
            );
            // Query form: act = 0, capture old.
            let mut old = core::mem::MaybeUninit::<SigAction>::uninit();
            assert_eq!(handle(15, 0, old.as_mut_ptr() as u64, KERNEL_SIGSET_SIZE as u64), 0);
            let old = unsafe { old.assume_init() };
            assert!(old.is_ignored());
            // Still SIG_IGN — the query didn't change it.
            current_process_signals(|s| {
                assert!(s.unwrap().action(15).unwrap().is_ignored());
            });
        });
    }

    /// `oldact == 0` installs without reporting the old disposition.
    #[test]
    fn null_oldact_installs_without_reporting() {
        with_process(|| {
            let h = SigAction {
                handler: 0x1234,
                flags: 0,
                restorer: 0,
                mask: 0,
            };
            assert_eq!(
                handle(3, &h as *const SigAction as u64, 0, KERNEL_SIGSET_SIZE as u64),
                0
            );
            current_process_signals(|s| {
                assert_eq!(s.unwrap().action(3).unwrap().handler, 0x1234);
            });
        });
    }

    /// A wrong `sigsetsize` (not 8) returns -EINVAL — the kernel
    /// validates the set width.
    #[test]
    fn wrong_sigsetsize_returns_einval() {
        with_process(|| {
            let h = SigAction::default_action();
            assert_eq!(handle(2, &h as *const SigAction as u64, 0, 16), -EINVAL);
            assert_eq!(handle(2, &h as *const SigAction as u64, 0, 4), -EINVAL);
            assert_eq!(handle(2, &h as *const SigAction as u64, 0, 0), -EINVAL);
        });
    }

    /// Out-of-range signal numbers return -EINVAL.
    #[test]
    fn out_of_range_signum_returns_einval() {
        with_process(|| {
            let h = SigAction::default_action();
            let p = &h as *const SigAction as u64;
            assert_eq!(handle(0, p, 0, KERNEL_SIGSET_SIZE as u64), -EINVAL);
            assert_eq!(handle(65, p, 0, KERNEL_SIGSET_SIZE as u64), -EINVAL);
            assert_eq!(handle(-1, p, 0, KERNEL_SIGSET_SIZE as u64), -EINVAL);
        });
    }

    /// SIGKILL (9) and SIGSTOP (19) cannot have their disposition
    /// changed — -EINVAL, and the disposition stays default.
    #[test]
    fn sigkill_sigstop_disposition_change_rejected() {
        with_process(|| {
            let h = SigAction {
                handler: 0xdead,
                flags: 0,
                restorer: 0,
                mask: 0,
            };
            let p = &h as *const SigAction as u64;
            assert_eq!(handle(SIGKILL, p, 0, KERNEL_SIGSET_SIZE as u64), -EINVAL);
            assert_eq!(handle(SIGSTOP, p, 0, KERNEL_SIGSET_SIZE as u64), -EINVAL);
            // Disposition unchanged (still default) for both.
            current_process_signals(|s| {
                let s = s.unwrap();
                assert!(s.action(SIGKILL).unwrap().is_default());
                assert!(s.action(SIGSTOP).unwrap().is_default());
            });
        });
    }

    /// With no current process installed, `rt_sigaction` returns -ESRCH
    /// (after the cheap range/size checks pass).
    #[test]
    fn no_process_returns_esrch() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();
        let h = SigAction::default_action();
        // signum + sigsetsize valid, but no process installed.
        let r = handle(2, &h as *const SigAction as u64, 0, KERNEL_SIGSET_SIZE as u64);
        assert_eq!(r, -ESRCH);
    }

    /// Dispatch wiring: `dispatch(SYS_RT_SIGACTION, ...)` routes here.
    #[test]
    fn dispatch_routes_rt_sigaction() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        use crate::syscall::dispatch::{dispatch, SYS_RT_SIGACTION};
        let address_space = AddressSpace::new(0x40_1000);
        current_process_install(Process::new(7, address_space));
        let new = SigAction {
            handler: 0x9999,
            flags: 0,
            restorer: 0,
            mask: 0,
        };
        let mut old = core::mem::MaybeUninit::<SigAction>::uninit();
        let r = dispatch(
            SYS_RT_SIGACTION,
            4, // SIGILL
            &new as *const SigAction as u64,
            old.as_mut_ptr() as u64,
            KERNEL_SIGSET_SIZE as u64,
            0,
            0,
        );
        assert_eq!(r, 0);
        let old = unsafe { old.assume_init() };
        assert!(old.is_default());
        current_process_uninstall();
    }
}
