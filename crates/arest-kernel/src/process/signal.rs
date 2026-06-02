// crates/arest-kernel/src/process/signal.rs
//
// Per-process signal state — the foundation for the signal family
// (#548 this slice; #549 SIGTERM/SIGKILL, #550 SIGSEGV, #551 SIGCHLD
// build on it). Three pieces of state, mirroring the three syscalls
// #548 ships:
//
//   * `actions` — the per-signal disposition table (`rt_sigaction`).
//     One `SigAction` per signal number 1..=MAX_SIGNAL. The kernel
//     consults this when a signal is delivered (#549+) to decide
//     whether to run a handler, ignore the signal, or take the default
//     action.
//   * `blocked` — the thread signal mask (`rt_sigprocmask`). A 64-bit
//     set; bit (signum-1) set ⇒ that signal is blocked (held pending
//     rather than delivered). Tier-1 is single-threaded so the
//     "thread" mask lives on the Process; when the scheduler grows
//     POSIX threads (#560+) it migrates to the per-thread struct
//     unchanged.
//   * `saved_context` — the slot `rt_sigreturn` restores from. When
//     the kernel delivers a signal to a handler it pushes the
//     interrupted register context here (and onto the user stack);
//     the handler's `rt_sigreturn` epilogue pops it to resume the
//     interrupted code. See the `rt_sigreturn` note below for why the
//     live-register half is gated on the ring-3 / delivery track.
//
// Why a cell-aligned model (predicate readings, not procedural cases)
// -------------------------------------------------------------------
// The AREST mandate is "predicate readings over procedural special-
// casing". The disposition of a signal is a *reading*: "signal N has
// disposition D". So the table is a flat array indexed by signal
// number, and the syscall handlers are thin — they read/write a cell
// rather than branch on signal identity. `record_into_cells` projects
// the live state as `Process_has_SignalAction` / `Process_has_SigMask`
// facts, the same shape `AddressSpace::record_into_cells` and
// `Process::record_into_cells` already emit, so a cell inspector sees
// the signal world the same way it sees the fd table.
//
// Why the kernel `struct sigaction` (k_sigaction), not the libc one
// ------------------------------------------------------------------
// The *raw* `rt_sigaction` syscall sees a different struct than the
// libc `struct sigaction`: musl's `__libc_sigaction`
// (`vendor/musl/src/signal/sigaction.c`) marshals the libc struct into
// `struct k_sigaction` (`vendor/musl/arch/x86_64/ksigaction.h`) before
// the `__syscall(SYS_rt_sigaction, ...)`. The kernel struct on x86_64
// is exactly 32 bytes:
//
//   offset  0: handler  (void (*)(int))  — 8 bytes
//   offset  8: flags    (unsigned long)  — 8 bytes
//   offset 16: restorer (void (*)(void)) — 8 bytes
//   offset 24: mask     (unsigned[2])    — 8 bytes (the 64-bit sigset)
//
// `SigAction` below is `repr(C)` with exactly that layout so a
// `core::ptr::read` of the userspace `act` pointer (under the tier-1
// identity mapping — same model as `ioctl` / `arch_prctl`) lands the
// fields where the ABI says.
//
// Signal-number range
// -------------------
// Linux `_NSIG` is 65 (`vendor/musl/arch/x86_64/bits/signal.h:152`),
// i.e. signal numbers 1..=64 are valid (`_NSIG-1`). The raw syscall's
// fourth argument `sigsetsize` is `_NSIG/8 = 8` bytes; the kernel
// rejects any other size with -EINVAL. We model the same: the action
// table is `MAX_SIGNAL` (= 64) entries, signal numbers are 1-based,
// and the mask is a single `u64` (64 bits — one per signal).

use alloc::format;
use arest::ast::{cell_push, fact_from_pairs, Object};

/// Highest valid signal number. Linux `_NSIG` is 65 so signal numbers
/// run 1..=64 (`_NSIG - 1`). Source:
/// `vendor/musl/arch/x86_64/bits/signal.h:152` (`#define _NSIG 65`).
/// The action table is `MAX_SIGNAL` entries (index `signum - 1`); the
/// thread mask is a `u64` (one bit per signal, bit `signum - 1`).
pub const MAX_SIGNAL: usize = 64;

/// Size in bytes of the kernel sigset the raw `rt_sigaction` /
/// `rt_sigprocmask` syscalls pass as their fourth `sigsetsize`
/// argument. Linux computes this as `_NSIG / 8 = 65 / 8 = 8` (integer
/// division; the 64 real signals fit in 8 bytes). The kernel rejects
/// any other value with -EINVAL — musl always passes exactly this
/// (`__syscall(SYS_rt_sigaction, sig, ..., _NSIG/8)` in
/// `vendor/musl/src/signal/sigaction.c`).
pub const KERNEL_SIGSET_SIZE: usize = 8;

/// `SIG_DFL` — the "take the default action" handler sentinel. Value 0.
/// Source: `vendor/musl/include/signal.h:284`
/// (`#define SIG_DFL ((void (*)(int)) 0)`). A `SigAction` whose
/// `handler` is 0 means "default disposition" (terminate / ignore /
/// stop / core, per the signal — that table lands with #549/#550).
pub const SIG_DFL: u64 = 0;

/// `SIG_IGN` — the "ignore this signal" handler sentinel. Value 1.
/// Source: `vendor/musl/include/signal.h:285`
/// (`#define SIG_IGN ((void (*)(int)) 1)`). A `SigAction` whose
/// `handler` is 1 means the signal is silently discarded on delivery.
pub const SIG_IGN: u64 = 1;

/// Kernel-ABI `struct k_sigaction` for x86_64 — the struct the *raw*
/// `rt_sigaction` syscall reads/writes (NOT the libc `struct
/// sigaction`, which musl marshals into this before the syscall). 32
/// bytes, `repr(C)`, field order + offsets per
/// `vendor/musl/arch/x86_64/ksigaction.h`:
///
///   handler  @ 0  — function pointer, or SIG_DFL (0) / SIG_IGN (1)
///   flags    @ 8  — SA_* bitset (SA_RESTART, SA_SIGINFO, SA_RESTORER…)
///   restorer @ 16 — the signal-trampoline return address (libc fills
///                   this with `__restore_rt`; the kernel uses it as
///                   the return address it pushes so the handler's
///                   `ret` lands on the `rt_sigreturn` stub)
///   mask     @ 24 — the 64-bit sigset blocked *during* the handler
///                   (on top of the thread mask), as `unsigned[2]`;
///                   modelled as one `u64` (little-endian: [0] is the
///                   low 32 bits, [1] the high 32 — a single u64 read
///                   is layout-identical on x86_64).
///
/// `Copy` so the table can store + hand back old actions by value
/// without lifetime ceremony — same rationale as `FdEntry`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigAction {
    /// Handler address, or the `SIG_DFL` (0) / `SIG_IGN` (1) sentinel.
    pub handler: u64,
    /// `SA_*` flags bitset (`vendor/musl/arch/x86_64/bits/signal.h`).
    /// Stored verbatim and handed back on the next `rt_sigaction`'s
    /// `oldact` — the kernel doesn't interpret most flags until
    /// delivery (#549+); the foundation just round-trips them.
    pub flags: u64,
    /// Signal-return trampoline address (libc's `__restore_rt`). The
    /// kernel pushes this as the handler's return address at delivery
    /// time so the handler's terminal `ret` enters `rt_sigreturn`.
    /// Round-tripped verbatim in tier-1.
    pub restorer: u64,
    /// Signals blocked for the duration of the handler, on top of the
    /// thread mask. Bit `(signum - 1)` set ⇒ blocked. `unsigned[2]`
    /// in C; one `u64` here (layout-identical on little-endian x86_64).
    pub mask: u64,
}

impl SigAction {
    /// The default disposition: `SIG_DFL`, no flags, no restorer, empty
    /// mask. This is what every signal starts at (Linux boots a process
    /// with every signal at SIG_DFL) and what `default_table` fills the
    /// action table with.
    pub const fn default_action() -> Self {
        Self {
            handler: SIG_DFL,
            flags: 0,
            restorer: 0,
            mask: 0,
        }
    }

    /// True when this action's handler is the `SIG_DFL` sentinel (0).
    /// A *predicate reading* of the disposition — the delivery path
    /// (#549+) reads `is_default()` rather than re-deriving "handler
    /// == 0" at each call site.
    pub fn is_default(&self) -> bool {
        self.handler == SIG_DFL
    }

    /// True when this action's handler is the `SIG_IGN` sentinel (1).
    /// Predicate reading — the delivery path discards the signal when
    /// `is_ignored()` holds.
    pub fn is_ignored(&self) -> bool {
        self.handler == SIG_IGN
    }

    /// True when a real (non-sentinel) userspace handler is installed
    /// (handler address > 1). The delivery path runs the handler when
    /// `has_handler()` holds.
    pub fn has_handler(&self) -> bool {
        self.handler > SIG_IGN
    }
}

/// A saved register context — the data `rt_sigreturn` restores. When
/// the kernel delivers a signal to a handler (#549+ delivery track) it
/// snapshots the interrupted thread's general-purpose registers + the
/// pre-signal blocked mask here (and mirrors them onto the user stack
/// as the `ucontext`'s `mcontext`); `rt_sigreturn` pops the slot to
/// resume the interrupted instruction stream.
///
/// Tier-1 stores the *bookkeeping* (which mask was active, whether a
/// frame is live) so the plumbing is testable without a VM; the
/// live general-register restore is gated on the ring-3 / delivery
/// track — see the `signal::SavedContext` field on `SignalState` and
/// the `rt_sigreturn` handler docstring. The register fields mirror
/// `struct sigcontext` (`vendor/musl/arch/x86_64/bits/signal.h:73`)
/// for the subset the foundation needs to round-trip; the full
/// `_fpstate` (xmm/x87) lands with #550's fault-delivery track.
///
/// `Copy` so the slot can be taken + handed back by value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedContext {
    /// The thread signal mask that was active *before* the signal was
    /// delivered. `rt_sigreturn` restores this as the live mask
    /// (signal delivery temporarily ORs in the handler's `sa_mask` +
    /// the delivered signal; the return restores the pre-delivery set).
    pub saved_mask: u64,
    /// Saved instruction pointer of the interrupted code (the address
    /// delivery will resume at). Mirrors `sigcontext.rip`.
    pub rip: u64,
    /// Saved stack pointer of the interrupted code. Mirrors
    /// `sigcontext.rsp`.
    pub rsp: u64,
}

/// Per-process (tier-1: per-process == per-thread) signal state. Owns
/// the disposition table, the thread mask, and the saved-context slot.
/// Lives inline on `Process` (same as `fd_table` / `fs_base`) rather
/// than in a separate global table, because every piece of it is
/// strictly per-process — unlike the futex table, no signal state is
/// shared kernel-wide.
#[derive(Debug, Clone)]
pub struct SignalState {
    /// Disposition table, indexed by `signum - 1` (signal numbers are
    /// 1-based; `actions[0]` is signal 1 = SIGHUP). Every entry starts
    /// at `SigAction::default_action()` (SIG_DFL) — Linux boots a
    /// process with all signals at their default disposition.
    actions: [SigAction; MAX_SIGNAL],
    /// Thread signal mask. Bit `(signum - 1)` set ⇒ that signal is
    /// blocked. Starts empty (0) — a freshly-spawned process blocks
    /// nothing (the libc start-up later blocks the implementation-
    /// internal signals via `rt_sigprocmask`).
    blocked: u64,
    /// Saved-context slot for `rt_sigreturn`. `None` when no signal
    /// handler is currently executing (the common case); `Some` while
    /// a handler runs, holding the context delivery snapshotted. The
    /// `rt_sigreturn` handler takes it (restoring the saved mask) and
    /// leaves `None`.
    saved_context: Option<SavedContext>,
}

impl Default for SignalState {
    fn default() -> Self {
        Self::new()
    }
}

impl SignalState {
    /// Construct the boot state: every signal at SIG_DFL, empty mask,
    /// no handler executing. This is what `Process::new` installs.
    pub fn new() -> Self {
        Self {
            actions: [SigAction::default_action(); MAX_SIGNAL],
            blocked: 0,
            saved_context: None,
        }
    }

    /// True when `signum` is a valid signal number (1..=MAX_SIGNAL).
    /// Signal 0 is the "null signal" used by `kill(pid, 0)` for
    /// existence checks — it has no disposition (you can't install a
    /// handler for it), so it's out of range here. Numbers above
    /// MAX_SIGNAL are invalid.
    pub fn is_valid_signum(signum: i32) -> bool {
        signum >= 1 && signum as usize <= MAX_SIGNAL
    }

    /// Read the current disposition for `signum`. Returns `None` for an
    /// out-of-range signal (caller maps to -EINVAL). A *reading*: "what
    /// is signal N's disposition right now".
    pub fn action(&self, signum: i32) -> Option<SigAction> {
        if !Self::is_valid_signum(signum) {
            return None;
        }
        Some(self.actions[(signum - 1) as usize])
    }

    /// Install `action` as the disposition for `signum`, returning the
    /// PREVIOUS disposition (so `rt_sigaction` can fill `oldact`).
    /// Returns `None` (and changes nothing) for an out-of-range signal.
    ///
    /// SIGKILL (9) and SIGSTOP (19) cannot have their disposition
    /// changed on Linux — `rt_sigaction` returns -EINVAL for them. That
    /// guard lives in the syscall handler (it needs to reject the call,
    /// not silently no-op); this method is the pure storage primitive
    /// and trusts the caller to have screened the un-catchable signals.
    pub fn set_action(&mut self, signum: i32, action: SigAction) -> Option<SigAction> {
        if !Self::is_valid_signum(signum) {
            return None;
        }
        let idx = (signum - 1) as usize;
        let old = self.actions[idx];
        self.actions[idx] = action;
        Some(old)
    }

    /// The current thread signal mask (bit `signum-1` set ⇒ blocked).
    pub fn blocked_mask(&self) -> u64 {
        self.blocked
    }

    /// True when `signum` is currently blocked in the thread mask. A
    /// predicate reading the delivery path (#549+) uses to decide
    /// whether to deliver now or hold the signal pending.
    pub fn is_blocked(&self, signum: i32) -> bool {
        if !Self::is_valid_signum(signum) {
            return false;
        }
        self.blocked & (1u64 << (signum - 1)) != 0
    }

    /// Apply an `rt_sigprocmask`-style mask update, returning the
    /// PREVIOUS mask (for `oldset`). `how` is one of `SIG_BLOCK` (0,
    /// OR the bits in), `SIG_UNBLOCK` (1, AND-NOT the bits out),
    /// `SIG_SETMASK` (2, replace wholesale). Returns `None` for an
    /// unrecognised `how` (caller maps to -EINVAL).
    ///
    /// SIGKILL / SIGSTOP can never be blocked: Linux silently clears
    /// those bits from the resulting mask regardless of `how`. We do
    /// the same so a process that tries to block them via SIG_SETMASK
    /// still receives them — matching the kernel's behaviour (it
    /// masks `sigmask(SIGKILL) | sigmask(SIGSTOP)` out of every new
    /// blocked set).
    pub fn update_mask(&mut self, how: i32, set: u64) -> Option<u64> {
        let old = self.blocked;
        let next = match how {
            SIG_BLOCK => old | set,
            SIG_UNBLOCK => old & !set,
            SIG_SETMASK => set,
            _ => return None,
        };
        // SIGKILL (9) and SIGSTOP (19) are un-blockable — clear their
        // bits (signum-1 = 8 and 18) from any resulting mask.
        self.blocked = next & !UNBLOCKABLE_MASK;
        Some(old)
    }

    /// Replace the thread mask wholesale (no `how` arithmetic), used by
    /// the delivery path (#549+) when it installs the handler's
    /// `sa_mask` and restores it on `rt_sigreturn`. SIGKILL / SIGSTOP
    /// are still forced un-blocked. Returns the previous mask.
    pub fn replace_mask(&mut self, mask: u64) -> u64 {
        let old = self.blocked;
        self.blocked = mask & !UNBLOCKABLE_MASK;
        old
    }

    /// True when a signal handler is currently executing (a saved
    /// context is parked, waiting for `rt_sigreturn`). The delivery
    /// path checks this to decide nesting behaviour; the foundation
    /// exposes it for the unit tests + the `rt_sigreturn` handler.
    pub fn in_handler(&self) -> bool {
        self.saved_context.is_some()
    }

    /// Park `ctx` as the saved context to restore on the next
    /// `rt_sigreturn`. Called by the delivery path (#549+) just before
    /// it redirects execution to the handler. Returns the previously-
    /// parked context if a handler was already running (nested signal),
    /// which the delivery path chains onto the user stack.
    pub fn push_context(&mut self, ctx: SavedContext) -> Option<SavedContext> {
        self.saved_context.replace(ctx)
    }

    /// Take the saved context (for `rt_sigreturn`): returns the parked
    /// `SavedContext` and clears the slot, AND restores the saved
    /// thread mask as the live mask (signal delivery temporarily ORs
    /// the handler's `sa_mask` + the delivered signal into the mask;
    /// the return restores the pre-delivery set). Returns `None` when
    /// no handler was executing — `rt_sigreturn` called outside a
    /// handler is undefined on Linux (it consumes whatever garbage is
    /// on the user stack); the foundation reports `None` so the handler
    /// can take the documented tier-1 path (return the saved rax, or 0).
    pub fn pop_context(&mut self) -> Option<SavedContext> {
        let taken = self.saved_context.take();
        if let Some(ctx) = taken {
            // Restore the pre-delivery mask (SIGKILL/SIGSTOP forced
            // un-blocked, as always).
            self.blocked = ctx.saved_mask & !UNBLOCKABLE_MASK;
        }
        taken
    }

    /// Compose this signal state's facts onto `state` and return the
    /// new state. Same shape as `AddressSpace::record_into_cells` /
    /// `Process::record_into_cells` — a pure projection the caller can
    /// commit via `system::apply` or inspect in tests.
    ///
    /// Cells emitted:
    ///   * `Process_has_SigMask` — (Process, Mask) where Mask is the
    ///     blocked set as a hex string (`"0"` when nothing blocked).
    ///   * `Process_has_SignalAction` — (Process, Signal, Disposition)
    ///     one fact per signal whose disposition is NOT the default
    ///     (SIG_DFL). Default entries elide (sparse projection — same
    ///     rationale as `Process::record_into_cells` eliding `Closed`
    ///     fd slots). Disposition is `"Ignored"` for SIG_IGN, or the
    ///     hex handler address for a real handler.
    ///   * `Process_has_SignalHandlerActive` — (Process, "true") iff a
    ///     handler is currently executing (a saved context is parked).
    ///     Elided when no handler runs.
    pub fn record_into_cells(&self, process_id: &str, state: &Object) -> Object {
        let mask_atom = format!("{:x}", self.blocked);
        let mut s = cell_push(
            "Process_has_SigMask",
            fact_from_pairs(&[("Process", process_id), ("Mask", &mask_atom)]),
            state,
        );
        for (idx, action) in self.actions.iter().enumerate() {
            if action.is_default() {
                // Sparse: default-disposition signals don't earn a fact.
                continue;
            }
            let signum = idx + 1;
            let signal_atom = format!("{}", signum);
            let disposition_atom = if action.is_ignored() {
                alloc::string::String::from("Ignored")
            } else {
                // Real handler — record its address (predicate reading:
                // "signal N is handled at address H").
                format!("{:x}", action.handler)
            };
            s = cell_push(
                "Process_has_SignalAction",
                fact_from_pairs(&[
                    ("Process", process_id),
                    ("Signal", &signal_atom),
                    ("Disposition", &disposition_atom),
                ]),
                &s,
            );
        }
        if self.in_handler() {
            s = cell_push(
                "Process_has_SignalHandlerActive",
                fact_from_pairs(&[("Process", process_id), ("Active", "true")]),
                &s,
            );
        }
        s
    }
}

/// `rt_sigprocmask` `how` value: OR the supplied bits into the mask.
/// Source: `vendor/musl/include/signal.h:30` (`#define SIG_BLOCK 0`).
pub const SIG_BLOCK: i32 = 0;

/// `rt_sigprocmask` `how` value: clear the supplied bits from the mask.
/// Source: `vendor/musl/include/signal.h:31` (`#define SIG_UNBLOCK 1`).
pub const SIG_UNBLOCK: i32 = 1;

/// `rt_sigprocmask` `how` value: replace the mask with the supplied
/// set. Source: `vendor/musl/include/signal.h:32`
/// (`#define SIG_SETMASK 2`).
pub const SIG_SETMASK: i32 = 2;

/// SIGKILL signal number. Source:
/// `vendor/musl/arch/x86_64/bits/signal.h` (`#define SIGKILL 9`).
/// Un-catchable + un-blockable — `rt_sigaction` refuses to change its
/// disposition and `rt_sigprocmask` refuses to block it.
pub const SIGKILL: i32 = 9;

/// SIGSTOP signal number. Source:
/// `vendor/musl/arch/x86_64/bits/signal.h` (`#define SIGSTOP 19`).
/// Un-catchable + un-blockable, same as SIGKILL.
pub const SIGSTOP: i32 = 19;

/// Bitmask of the un-blockable / un-catchable signals (SIGKILL +
/// SIGSTOP), in `signum - 1` bit positions. Cleared from every
/// resulting blocked mask and rejected by `rt_sigaction`'s disposition
/// change. `1 << 8` (SIGKILL=9) | `1 << 18` (SIGSTOP=19).
pub const UNBLOCKABLE_MASK: u64 = (1u64 << (SIGKILL - 1)) | (1u64 << (SIGSTOP - 1));

#[cfg(test)]
mod tests {
    use super::*;

    /// `MAX_SIGNAL` is 64 — Linux `_NSIG` (65) minus the 1-based
    /// off-by-one, matching `vendor/musl/arch/x86_64/bits/signal.h`.
    #[test]
    fn max_signal_matches_nsig() {
        assert_eq!(MAX_SIGNAL, 64);
    }

    /// `KERNEL_SIGSET_SIZE` is 8 (`_NSIG/8`) — the raw syscall's
    /// fourth `sigsetsize` argument value.
    #[test]
    fn kernel_sigset_size_is_eight() {
        assert_eq!(KERNEL_SIGSET_SIZE, 8);
    }

    /// `SigAction` is exactly 32 bytes — the x86_64 kernel
    /// `struct k_sigaction` ABI (`vendor/musl/arch/x86_64/ksigaction.h`:
    /// handler(8) + flags(8) + restorer(8) + mask(8)).
    #[test]
    fn sigaction_struct_is_32_bytes() {
        assert_eq!(core::mem::size_of::<SigAction>(), 32);
    }

    /// SIG_DFL / SIG_IGN sentinels match the musl uapi values (0 / 1).
    #[test]
    fn sig_dfl_ign_sentinels_match_uapi() {
        assert_eq!(SIG_DFL, 0);
        assert_eq!(SIG_IGN, 1);
    }

    /// SIG_BLOCK / SIG_UNBLOCK / SIG_SETMASK match the musl uapi values.
    #[test]
    fn sigprocmask_how_constants_match_uapi() {
        assert_eq!(SIG_BLOCK, 0);
        assert_eq!(SIG_UNBLOCK, 1);
        assert_eq!(SIG_SETMASK, 2);
    }

    /// A fresh `SignalState` has every signal at SIG_DFL, empty mask,
    /// no handler executing.
    #[test]
    fn new_state_is_all_default() {
        let s = SignalState::new();
        assert_eq!(s.blocked_mask(), 0);
        assert!(!s.in_handler());
        for signum in 1..=MAX_SIGNAL as i32 {
            let a = s.action(signum).expect("valid signum");
            assert!(a.is_default(), "signal {} must start at SIG_DFL", signum);
        }
    }

    /// `is_valid_signum` accepts 1..=64, rejects 0 and 65+.
    #[test]
    fn valid_signum_range() {
        assert!(!SignalState::is_valid_signum(0));
        assert!(SignalState::is_valid_signum(1));
        assert!(SignalState::is_valid_signum(64));
        assert!(!SignalState::is_valid_signum(65));
        assert!(!SignalState::is_valid_signum(-1));
    }

    /// `set_action` installs the new disposition and returns the old
    /// one — the round-trip `rt_sigaction`'s `oldact` depends on.
    #[test]
    fn set_action_returns_previous() {
        let mut s = SignalState::new();
        let handler = SigAction {
            handler: 0xdead_beef,
            flags: 0,
            restorer: 0,
            mask: 0,
        };
        // First install returns the default (SIG_DFL).
        let old = s.set_action(2, handler).expect("valid signum");
        assert!(old.is_default());
        // The new disposition is now live.
        assert_eq!(s.action(2).unwrap(), handler);
        // Installing again returns the previously-installed handler.
        let ign = SigAction {
            handler: SIG_IGN,
            ..SigAction::default_action()
        };
        let old2 = s.set_action(2, ign).expect("valid signum");
        assert_eq!(old2, handler);
        assert!(s.action(2).unwrap().is_ignored());
    }

    /// `set_action` against an out-of-range signal returns None and
    /// leaves the table untouched.
    #[test]
    fn set_action_out_of_range_is_none() {
        let mut s = SignalState::new();
        assert!(s.set_action(0, SigAction::default_action()).is_none());
        assert!(s.set_action(65, SigAction::default_action()).is_none());
    }

    /// SIG_BLOCK ORs bits in; the returned old mask reflects the
    /// pre-update state.
    #[test]
    fn update_mask_block_ors_bits() {
        let mut s = SignalState::new();
        // Block SIGINT (2) → bit 1.
        let old = s.update_mask(SIG_BLOCK, 1 << 1).expect("valid how");
        assert_eq!(old, 0);
        assert!(s.is_blocked(2));
        assert_eq!(s.blocked_mask(), 1 << 1);
        // Block SIGHUP (1) → bit 0; SIGINT stays blocked.
        let old2 = s.update_mask(SIG_BLOCK, 1 << 0).expect("valid how");
        assert_eq!(old2, 1 << 1);
        assert!(s.is_blocked(1));
        assert!(s.is_blocked(2));
    }

    /// SIG_UNBLOCK clears the supplied bits.
    #[test]
    fn update_mask_unblock_clears_bits() {
        let mut s = SignalState::new();
        s.update_mask(SIG_SETMASK, (1 << 1) | (1 << 2)).unwrap();
        assert!(s.is_blocked(2));
        assert!(s.is_blocked(3));
        let old = s.update_mask(SIG_UNBLOCK, 1 << 1).expect("valid how");
        assert_eq!(old, (1 << 1) | (1 << 2));
        assert!(!s.is_blocked(2));
        assert!(s.is_blocked(3));
    }

    /// SIG_SETMASK replaces the mask wholesale.
    #[test]
    fn update_mask_setmask_replaces() {
        let mut s = SignalState::new();
        s.update_mask(SIG_BLOCK, 1 << 5).unwrap();
        let old = s.update_mask(SIG_SETMASK, 1 << 1).expect("valid how");
        assert_eq!(old, 1 << 5);
        assert!(!s.is_blocked(6));
        assert!(s.is_blocked(2));
    }

    /// An unrecognised `how` returns None (→ -EINVAL at the handler).
    #[test]
    fn update_mask_unknown_how_is_none() {
        let mut s = SignalState::new();
        assert!(s.update_mask(99, 0).is_none());
        assert!(s.update_mask(-1, 0).is_none());
    }

    /// SIGKILL (9) and SIGSTOP (19) can never be blocked — even an
    /// explicit SIG_SETMASK that includes their bits leaves them
    /// unblocked, matching the Linux kernel's forced clear.
    #[test]
    fn sigkill_sigstop_cannot_be_blocked() {
        let mut s = SignalState::new();
        // Try to block EVERYTHING via SIG_SETMASK.
        s.update_mask(SIG_SETMASK, u64::MAX).unwrap();
        assert!(!s.is_blocked(SIGKILL), "SIGKILL must never be blockable");
        assert!(!s.is_blocked(SIGSTOP), "SIGSTOP must never be blockable");
        // Every other signal IS blocked.
        assert!(s.is_blocked(1));
        assert!(s.is_blocked(2));
        assert!(s.is_blocked(64));
    }

    /// `push_context` / `pop_context` round-trip: parking a context
    /// marks `in_handler`, popping it restores the saved mask and
    /// clears the slot.
    #[test]
    fn saved_context_round_trips_and_restores_mask() {
        let mut s = SignalState::new();
        assert!(!s.in_handler());
        // Pretend delivery: the pre-signal mask was bit-3 set.
        let ctx = SavedContext {
            saved_mask: 1 << 3,
            rip: 0x40_1000,
            rsp: 0x7fff_0000,
        };
        // Delivery would also widen the live mask; simulate that.
        s.replace_mask(0xff);
        assert!(s.push_context(ctx).is_none());
        assert!(s.in_handler());
        // rt_sigreturn pops it → restores saved_mask (bit 3), clears slot.
        let popped = s.pop_context().expect("a context was parked");
        assert_eq!(popped, ctx);
        assert!(!s.in_handler());
        assert_eq!(s.blocked_mask(), 1 << 3);
    }

    /// `pop_context` outside a handler returns None and changes nothing.
    #[test]
    fn pop_context_without_handler_is_none() {
        let mut s = SignalState::new();
        assert!(s.pop_context().is_none());
        assert!(!s.in_handler());
    }

    /// `record_into_cells` emits a SigMask fact and one SignalAction
    /// fact per non-default signal; default signals elide.
    #[test]
    fn record_into_cells_emits_mask_and_nondefault_actions() {
        let mut s = SignalState::new();
        // Block SIGINT, install a handler for SIGTERM (15), ignore
        // SIGHUP (1).
        s.update_mask(SIG_BLOCK, 1 << 1).unwrap();
        s.set_action(
            15,
            SigAction {
                handler: 0xcafe_babe,
                flags: 0,
                restorer: 0,
                mask: 0,
            },
        )
        .unwrap();
        s.set_action(
            1,
            SigAction {
                handler: SIG_IGN,
                ..SigAction::default_action()
            },
        )
        .unwrap();
        let recorded = s.record_into_cells("proc7", &Object::phi());
        let serialised = format!("{:?}", recorded);
        assert!(serialised.contains("Process_has_SigMask"));
        assert!(serialised.contains("Process_has_SignalAction"));
        // SIGTERM's handler address shows up in hex.
        assert!(serialised.contains("cafebabe"));
        // SIGHUP's Ignored disposition shows up.
        assert!(serialised.contains("Ignored"));
        // Exactly two SignalAction facts (SIGHUP ignored + SIGTERM
        // handled); the other 62 default signals elide. Count the
        // unique `Disposition` pair occurrences.
        let count = serialised.matches("Disposition").count();
        assert_eq!(count, 2, "only non-default signals earn a SignalAction fact");
    }

    /// `record_into_cells` emits the handler-active fact only while a
    /// handler is executing.
    #[test]
    fn record_into_cells_handler_active_when_in_handler() {
        let mut s = SignalState::new();
        let no_handler = format!("{:?}", s.record_into_cells("p", &Object::phi()));
        assert!(!no_handler.contains("Process_has_SignalHandlerActive"));
        s.push_context(SavedContext {
            saved_mask: 0,
            rip: 0,
            rsp: 0,
        });
        let active = format!("{:?}", s.record_into_cells("p", &Object::phi()));
        assert!(active.contains("Process_has_SignalHandlerActive"));
    }
}
