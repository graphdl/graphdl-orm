// crates/arest-kernel/src/process/trampoline.rs
//
// Privilege-transition trampoline — switches the CPU from kernel
// (ring 0 / EL1) to userspace (ring 3 / EL0) and jumps to a Linux
// process's entry point with `rsp` / `sp` pointing at the initial
// stack frame the `process::stack::StackBuilder` populated. The third
// piece of the #521 spawn pipeline (after #519's `AddressSpace`
// allocator and the just-landed `process::stack` builder).
//
// Why this is its own module
// --------------------------
// `process::process` owns the high-level Process struct + spawn
// orchestration; the actual asm shim that flips CPL bits + reloads
// segment selectors lives here so the unsafe / arch-specific surface
// is concentrated in one file rather than threaded through Process's
// constructor. Mirrors the `crate::doom` host-shim split: the trait
// + Linker-binding scaffolding lives in `doom.rs` while the per-frame
// `wasmi::Caller` plumbing stays close to the imports it serves
// (lines 855-940). Same shape — keep the asm + privilege bits in
// their own well-named module so `#[forbid(unsafe_code)]` candidates
// (the eventual safer Process API) can opt out cleanly.
//
// Why aarch64 / armv7 are stubs
// -----------------------------
// Tier-1 of the #521 spawn epic only ships an x86_64 trampoline;
// the aarch64 + armv7 paths need GICv3 / EL0 support that #344's
// arch arms haven't grown yet. The stubs return
// `TrampolineError::UnsupportedArch` so a caller that reaches the
// trampoline on those targets gets a typed error rather than a
// link-time symbol-not-found. Same deferred-implementation pattern
// the `crate::pci` and `crate::repl` modules use (x86_64-only,
// not even compiled on aarch64 — but trampoline must live on
// every arch because `Process::spawn` is arch-neutral).
//
// Why the actual ring-3 jump is `unimplemented!()` even on x86_64
// ----------------------------------------------------------------
// Reaching ring 3 needs three pieces #521 doesn't itself ship:
//
//   1. A GDT with userspace code + data segment descriptors. The
//      kernel today inherits the firmware's GDT (`arch::uefi::mod`
//      docstring lines 21-24); it has no DPL=3 descriptors.
//   2. A TSS with `rsp0` pointing at the kernel stack so syscall /
//      iret can switch back in. Same firmware-GDT story —
//      `arch::uefi::interrupts` line 43-46 documents the absence.
//   3. A TLB-stable mapping of the loaded segments + initial stack
//      into a userspace VA range. The `AddressSpace` model
//      (`process::address_space`) holds the bytes but doesn't yet
//      install page-table entries.
//
// All three land in #525 / #526 / #527 (sub-tasks of the #472 epic
// after #523 ld-musl). For tier-1 the trampoline ships a SETUP
// path (validates the inputs, computes the iretq stack frame the
// future jump will use) and an INVOKE path that's unimplemented
// for now — the structural pieces are tested, the ring-3 jump
// itself is gated behind a `unimplemented!()` until the GDT/TSS
// scaffolding lands.
//
// Why we don't gate on `cfg(target_arch = "x86_64")` for the SETUP
// ---------------------------------------------------------------
// The setup path computes a pure-data structure (an `IretqFrame`
// — five u64s laid out in System V iretq order); the math is
// arch-agnostic, only the actual `iretq` instruction is x86-only.
// Keeping the setup compileable on every arch lets the unit tests
// run as host-side `#[cfg(test)]` blocks regardless of which arch
// is the build target. The aarch64 / armv7 invoke shims are stubs
// per the rationale above.

use super::address_space::AddressSpace;
use super::stack::InitialStack;

/// Errors `setup` and `invoke` can return. Stays `Copy` so callers
/// can store + compare without lifetime hassles, matching
/// `process::address_space::LoaderError` and `process::stack::StackError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrampolineError {
    /// Entry point virtual address is zero. Almost certainly a
    /// caller bug — a real ELF binary's `e_entry` is in the loaded
    /// segment range; zero is what an uninitialised AddressSpace
    /// reports.
    NullEntry,
    /// Initial stack pointer doesn't satisfy 16-byte ABI alignment.
    /// Defensive — `StackBuilder::finalize` already enforces this,
    /// but `setup` re-checks so a future caller that constructs
    /// `InitialStack` by some other path can't smuggle in a
    /// misaligned SP.
    MisalignedStack,
    /// Trampoline asm is not implemented for the current target arch.
    /// aarch64 / armv7 paths — they need EL0 + page-table support
    /// that hasn't landed yet.
    UnsupportedArch,
    /// Trampoline asm is not yet implemented for the current arch
    /// because the prerequisites (GDT/TSS, page-table install) are
    /// pending. x86_64 returns this until #525/#526/#527 land.
    NotYetImplemented,
}

/// On-stack frame the x86_64 `iretq` instruction consumes. Five u64s
/// in this exact order (high-to-low address):
///
///   ┌──────────────┐ +32  rsp value (userspace stack)
///   │ ss           │ +24  userspace stack-segment selector (SS|RPL=3)
///   │ rsp          │ +16  userspace rsp (= stack.sp())
///   │ rflags       │ +8   IF=1, IOPL=0, reserved bits per the spec
///   │ cs           │ +0   userspace code-segment selector (CS|RPL=3)
///   │ rip          │ -8   userspace rip (= entry point)
///   └──────────────┘
///
/// `repr(C)` so the field layout matches the iretq stack-frame order
/// the CPU pops. All fields are `u64` to match x86_64 long-mode
/// iretq's 64-bit pop width.
///
/// Built by `setup_x86_64` against the AddressSpace + InitialStack +
/// the GDT selectors the future #526 will publish; consumed by an asm
/// shim that loads it into an actual stack frame and `iretq`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct IretqFrame {
    /// Userspace RIP — the CPU jumps here after iretq. Equals the
    /// `AddressSpace::entry_point` (= the ELF's `e_entry`).
    pub rip: u64,
    /// Userspace code-segment selector with RPL=3. Index into the
    /// future #526 GDT's user-code descriptor; for tier-1 the value
    /// is a placeholder until the GDT lands. The bottom three bits
    /// MUST be `0b011` (RPL=3 + TI=0) for the iretq to actually
    /// transition to ring 3 — the CPU checks this field's RPL
    /// against the current CPL.
    pub cs: u64,
    /// EFLAGS / RFLAGS. Per the SysV-ABI initial state for a
    /// freshly-spawned process: IF=1 (interrupts enabled in
    /// userspace), IOPL=0 (no port I/O privileges), DF=0 (forward
    /// string ops), reserved bit 1 = 1. Numeric value: 0x202.
    pub rflags: u64,
    /// Userspace RSP — the CPU loads this into rsp after iretq.
    /// Equals `InitialStack::sp()` from the StackBuilder.
    pub rsp: u64,
    /// Userspace stack-segment selector with RPL=3. Same shape as
    /// `cs` — index into the GDT's user-data descriptor.
    pub ss: u64,
}

/// EFLAGS / RFLAGS value for a freshly-spawned userspace process.
/// IF=1 (bit 9): interrupts enabled — the process can be preempted
/// by the timer IRQ. Reserved bit 1 = 1 per the spec. All other bits
/// = 0 (DF=0, IOPL=0, AC=0 etc.).
pub const USER_RFLAGS: u64 = 0x202;

/// Placeholder userspace code-segment selector (RPL=3, TI=0, index
/// will be assigned by the future #526 GDT module). The constant
/// carries the RPL bits so the IretqFrame's invariant holds even
/// before the real index is known. Format per the AMD64 manual:
///
///   bits  0..2 : RPL (3 = ring 3)
///   bit      2 : TI  (0 = GDT, 1 = LDT)
///   bits  3..15: descriptor index
///
/// 0x1B = 0b0001_1011 — index 3, GDT, RPL 3. The "3" index is what
/// the rust-osdev `bootloader` BIOS arm uses for its user-code
/// descriptor (`arch::x86_64::gdt::USER_CODE_SELECTOR`); we pick the
/// same value so a future migration doesn't churn the constant.
pub const PLACEHOLDER_USER_CS: u64 = 0x1B;

/// Placeholder userspace stack-segment selector. Same format as
/// `PLACEHOLDER_USER_CS` — index 4 (one slot after user code), GDT,
/// RPL 3. 0x23 = 0b0010_0011.
pub const PLACEHOLDER_USER_SS: u64 = 0x23;

/// Build the iretq frame the x86_64 trampoline will consume to
/// transition into the loaded process. Pure validation + layout —
/// no CPU instruction is executed; the frame is data the future
/// `invoke_x86_64` will load into a stack slot before iretq.
///
/// Validates:
///   * `entry_point != 0` — non-empty AddressSpace
///   * `sp() % 16 == 0` — System V ABI alignment
///
/// The CS / SS / RFLAGS values come from the per-arch placeholder
/// constants until #526 lands real GDT selectors. The frame is
/// `Copy` so the caller can stash it in the Process struct
/// independently of the IretqFrame's eventual on-stack home.
pub fn setup_x86_64(
    address_space: &AddressSpace,
    stack: &InitialStack,
) -> Result<IretqFrame, TrampolineError> {
    if address_space.entry_point == 0 {
        return Err(TrampolineError::NullEntry);
    }
    if stack.sp() % 16 != 0 {
        return Err(TrampolineError::MisalignedStack);
    }
    Ok(IretqFrame {
        rip: address_space.entry_point,
        cs: PLACEHOLDER_USER_CS,
        rflags: USER_RFLAGS,
        rsp: stack.sp(),
        ss: PLACEHOLDER_USER_SS,
    })
}

/// Invoke the trampoline — actually transition to ring 3 and jump
/// to the entry point. Diverges (returns `!`) when the jump
/// succeeds; returns `Err(...)` if the prerequisites aren't met.
///
/// On x86_64: not yet implemented. The setup path produces the
/// `IretqFrame`; the actual `iretq` shim needs the GDT/TSS
/// scaffolding from #526 + the page-table install from #527 before
/// the CPU can reach ring 3 without faulting. Returns
/// `TrampolineError::NotYetImplemented`.
///
/// On aarch64 / armv7: returns `TrampolineError::UnsupportedArch`.
/// The EL0 transition needs the arch::aarch64 / arch::armv7 arms
/// to grow EL1 → EL0 support (planned alongside the GICv3 driver).
///
/// SAFETY-WISE: when implemented, this function is a one-way ticket
/// — the kernel stack is unwound, all kernel locals are dropped,
/// and the CPU is in userspace. The caller must NOT hold any
/// resource that needs cleanup beyond what `Process::spawn` already
/// arranges.
#[cfg(target_arch = "x86_64")]
pub fn invoke(
    _address_space: &AddressSpace,
    _stack: &InitialStack,
) -> Result<core::convert::Infallible, TrampolineError> {
    // Validate the inputs via the setup path so a future caller can
    // call `invoke` directly and still get the same error surface
    // `setup_x86_64` produces. The frame is computed but discarded
    // here — the asm shim that consumes it lands in #526 alongside
    // the GDT/TSS scaffolding.
    let _frame = setup_x86_64(_address_space, _stack)?;
    Err(TrampolineError::NotYetImplemented)
}

/// aarch64 stub — see module docstring for the rationale. Same
/// signature as the x86_64 invoke so `Process::spawn` is arch-neutral.
#[cfg(target_arch = "aarch64")]
pub fn invoke(
    _address_space: &AddressSpace,
    _stack: &InitialStack,
) -> Result<core::convert::Infallible, TrampolineError> {
    Err(TrampolineError::UnsupportedArch)
}

/// armv7 stub — same shape as the aarch64 stub.
#[cfg(target_arch = "arm")]
pub fn invoke(
    _address_space: &AddressSpace,
    _stack: &InitialStack,
) -> Result<core::convert::Infallible, TrampolineError> {
    Err(TrampolineError::UnsupportedArch)
}

/// Host-target / unknown-arch stub. The kernel currently builds for
/// three target arches (x86_64-unknown-uefi, aarch64-unknown-uefi,
/// arest-kernel-armv7-uefi.json), each handled by an arm above; this
/// catch-all is here for two reasons: (a) future arch widening
/// (riscv64, ppc64le, s390x — all on Linux's roadmap for AREST
/// hosting) gets a typed `UnsupportedArch` error rather than a
/// link-time symbol-not-found, (b) any future host-side test harness
/// (the kernel's `[[bin]]` carries `test = false` today, but a
/// future `[[lib]]` slice could light up host-side `cargo test`)
/// resolves the symbol to a `UnsupportedArch` rather than failing
/// to link.
#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "arm",
)))]
pub fn invoke(
    _address_space: &AddressSpace,
    _stack: &InitialStack,
) -> Result<core::convert::Infallible, TrampolineError> {
    Err(TrampolineError::UnsupportedArch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::SegmentPerm;
    use crate::process::stack::StackBuilder;

    /// `setup_x86_64` happy path: build a minimal AddressSpace + stack
    /// and verify the IretqFrame fields match the inputs.
    #[test]
    fn setup_x86_64_happy_path() {
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        let stack = StackBuilder::new()
            .push_argv(b"/bin/true")
            .finalize()
            .expect("stack finalize");
        let frame = setup_x86_64(&address_space, &stack).expect("setup must succeed");
        assert_eq!(frame.rip, 0x40_1000);
        assert_eq!(frame.cs, PLACEHOLDER_USER_CS);
        assert_eq!(frame.rflags, USER_RFLAGS);
        assert_eq!(frame.rsp, stack.sp());
        assert_eq!(frame.ss, PLACEHOLDER_USER_SS);
    }

    /// Null entry point is rejected. An uninitialised AddressSpace
    /// reports entry_point = 0; the trampoline refuses to invoke
    /// against it.
    #[test]
    fn setup_x86_64_rejects_null_entry() {
        let address_space = AddressSpace::new(0);
        let stack = StackBuilder::new()
            .finalize()
            .expect("stack finalize");
        let err = setup_x86_64(&address_space, &stack).unwrap_err();
        assert_eq!(err, TrampolineError::NullEntry);
    }

    /// Userspace CS selector has RPL=3 (bottom three bits = 0b011).
    /// The CPU iretq's CPL check refuses any other RPL.
    #[test]
    fn placeholder_user_cs_has_rpl_3() {
        assert_eq!(PLACEHOLDER_USER_CS & 0b11, 3, "RPL must be 3");
    }

    /// Userspace SS selector has RPL=3 (same constraint as CS).
    #[test]
    fn placeholder_user_ss_has_rpl_3() {
        assert_eq!(PLACEHOLDER_USER_SS & 0b11, 3, "RPL must be 3");
    }

    /// Userspace RFLAGS has IF=1 (bit 9) so userspace can be
    /// preempted by the timer IRQ.
    #[test]
    fn user_rflags_has_if_set() {
        assert_eq!(USER_RFLAGS & (1 << 9), 1 << 9, "IF (bit 9) must be set");
    }

    /// Userspace RFLAGS has reserved bit 1 = 1 per the spec.
    #[test]
    fn user_rflags_reserved_bit_set() {
        assert_eq!(USER_RFLAGS & (1 << 1), 1 << 1, "reserved bit 1 must be set");
    }

    /// `IretqFrame` is repr(C) and 40 bytes — five u64s. Matches
    /// the on-stack layout `iretq` consumes.
    #[test]
    fn iretq_frame_size_is_40_bytes() {
        assert_eq!(core::mem::size_of::<IretqFrame>(), 40);
    }

    /// `invoke` returns `NotYetImplemented` on x86_64 (because the
    /// GDT/TSS prerequisites haven't landed) and `UnsupportedArch`
    /// on aarch64 / armv7 / host targets. Either way it returns
    /// an error rather than diverging.
    #[test]
    fn invoke_returns_not_implemented_or_unsupported() {
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        let stack = StackBuilder::new()
            .push_argv(b"/bin/true")
            .finalize()
            .expect("stack finalize");
        let result = invoke(&address_space, &stack);
        assert!(result.is_err(), "invoke must error until #526/#527 land");
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                TrampolineError::NotYetImplemented | TrampolineError::UnsupportedArch
            ),
            "expected NotYetImplemented or UnsupportedArch, got {:?}",
            err
        );
    }
}
