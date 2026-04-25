// crates/arest-kernel/src/arch/armv7/msvc_shims.rs
//
// MSVC-ARM CRT helper shims for the armv7-UEFI scaffold (#346).
//
// Why this file exists:
//
// The custom target JSON `arest-kernel-armv7-uefi.json` declares
// `is-like-msvc: true` + `llvm-target: thumbv7a-pc-windows-msvc`.
// That's the only way to get rust-lld to emit a PE32+ `.efi` for
// 32-bit ARM under the same MSVC linker flavor the existing
// `aarch64-unknown-uefi` target uses. Under that linker flavor LLVM's
// ARM backend lowers a handful of operations to Microsoft-CRT-named
// helper calls — `__rt_udiv`, `__rt_udiv64`, `__rt_sdiv`, `__chkstk`,
// `__u64tod`, `__u64tos`, `__i64tod`, `__i64tos`, `__dtoi64`,
// `__dtou64` — that the standard `compiler_builtins` crate only
// provides under their AEABI / cross-platform names (`__udivsi3`,
// `__udivdi3`, `__divsi3`, `__floatundidf`, `__floatundisf`,
// `__floatdidf`, `__floatdisf`, `__fixdfdi`, `__fixunsdfdi`).
//
// On Microsoft's own toolchain those CRT helpers come from
// `armrt.lib` / VC++ runtime. We don't link any MSVC CRT (a UEFI
// scaffold can't — there's no Windows host loader and no `vcruntime`
// import library in the rust-lld sysroot for ARM-UEFI), so the link
// fails with "undefined symbol: __rt_udiv" etc. unless we provide
// them. The aarch64-UEFI arm doesn't hit this because compiler_
// builtins's AArch64 backend already names its helpers the way
// LLVM-AArch64 calls them; only the ARM 32-bit backend has the
// MSVC-name divergence.
//
// Each shim below is a trivial wrapper that re-routes the MSVC name
// to the AEABI/cross-platform name compiler_builtins exports. The
// arg-order swap matters for `__rt_udiv` / `__rt_sdiv` (Microsoft
// passes `divisor, dividend`; the cross-platform helpers take
// `dividend, divisor`). `__chkstk` on ARM is a single push-jump
// frame the LLVM backend emits as a leaf call; we provide a no-op
// since the kernel scaffold has zero functions with frames bigger
// than 4 KiB (the threshold for stack-probe emission), and even in
// follow-ups the no-op is harmless until we have a guard page set
// up — at which point #346e installs the real probe.
//
// Gated behind `cfg(all(target_os = "uefi", target_arch = "arm"))`
// transitively (the parent `arch::armv7` module already carries the
// gate via `arch/mod.rs`).
//
// Reference: Microsoft "ARM ABI conventions" docs and LLVM's
// `lib/Target/ARM/ARMISelLowering.cpp::setLibcallName` for the MSVC
// ARM environment.

// The MSVC ARM ABI passes integer 64-bit values in a register pair
// the LLVM backend promotes to `extern "C"` `u64` / `i64` directly.
// Our shims follow the same `extern "C"` calling convention so the
// linker name mangling matches what the backend emits.

// ---------------------------------------------------------------------------
// Integer division (32-bit)
// ---------------------------------------------------------------------------

unsafe extern "C" {
    fn __udivsi3(n: u32, d: u32) -> u32;
    fn __divsi3(n: i32, d: i32) -> i32;
    fn __udivdi3(n: u64, d: u64) -> u64;
    fn __divdi3(n: i64, d: i64) -> i64;
    fn __floatundidf(i: u64) -> f64;
    fn __floatundisf(i: u64) -> f32;
    fn __floatdidf(i: i64) -> f64;
    fn __floatdisf(i: i64) -> f32;
    fn __fixdfdi(f: f64) -> i64;
    fn __fixunsdfdi(f: f64) -> u64;
}

/// MSVC ARM unsigned 32-bit divide. Microsoft argument order is
/// `(divisor, dividend)`, OPPOSITE of the standard `__udivsi3(n, d)`.
/// Returns quotient in `r0`. Re-routes to compiler_builtins.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __rt_udiv(divisor: u32, dividend: u32) -> u32 {
    unsafe { __udivsi3(dividend, divisor) }
}

/// MSVC ARM signed 32-bit divide. Same arg-order swap as `__rt_udiv`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __rt_sdiv(divisor: i32, dividend: i32) -> i32 {
    unsafe { __divsi3(dividend, divisor) }
}

/// MSVC ARM unsigned 64-bit divide. Same `(divisor, dividend)`
/// argument order as the 32-bit form.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __rt_udiv64(divisor: u64, dividend: u64) -> u64 {
    unsafe { __udivdi3(dividend, divisor) }
}

/// MSVC ARM signed 64-bit divide.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __rt_sdiv64(divisor: i64, dividend: i64) -> i64 {
    unsafe { __divdi3(dividend, divisor) }
}

// ---------------------------------------------------------------------------
// Float conversions (64-bit integer ↔ f32/f64)
// ---------------------------------------------------------------------------

/// MSVC ARM `u64 → f64`. Backed by compiler_builtins's
/// `__floatundidf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __u64tod(i: u64) -> f64 {
    unsafe { __floatundidf(i) }
}

/// MSVC ARM `u64 → f32`. Backed by compiler_builtins's
/// `__floatundisf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __u64tos(i: u64) -> f32 {
    unsafe { __floatundisf(i) }
}

/// MSVC ARM `i64 → f64`. Backed by compiler_builtins's `__floatdidf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __i64tod(i: i64) -> f64 {
    unsafe { __floatdidf(i) }
}

/// MSVC ARM `i64 → f32`. Backed by compiler_builtins's `__floatdisf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __i64tos(i: i64) -> f32 {
    unsafe { __floatdisf(i) }
}

/// MSVC ARM `f64 → i64`. Backed by compiler_builtins's `__fixdfdi`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __dtoi64(f: f64) -> i64 {
    unsafe { __fixdfdi(f) }
}

/// MSVC ARM `f64 → u64`. Backed by compiler_builtins's `__fixunsdfdi`
/// ("fix unsigned df di" — convert f64 to u64). Sibling of `__dtoi64`
/// for the unsigned destination type.
///
/// Why this shim exists: nightlies from 2026-04-22 onward route a
/// `compiler_builtins::libm_math::hypot::as_hypot_denorm` code path
/// through an f64→u64 cast that LLVM's ARM-MSVC backend lowers to
/// `__dtou64`. Before this shim, the arch::armv7 build had to be
/// pinned to nightly-2026-04-21 (or kept on the dev profile, which
/// dead-code-eliminates the path) to avoid an undefined-symbol link
/// error against `__dtou64`. With this shim the toolchain pin in
/// `Dockerfile.uefi-armv7` can drop back to plain `nightly` and the
/// `--release` profile becomes available again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __dtou64(f: f64) -> u64 {
    unsafe { __fixunsdfdi(f) }
}

// ---------------------------------------------------------------------------
// Stack probe (no-op)
// ---------------------------------------------------------------------------

/// MSVC ARM stack-allocation probe. LLVM's ARM backend emits a call
/// to `__chkstk` whenever a function's stack frame exceeds the
/// per-page threshold (4 KiB on Windows; same default under
/// `is-like-windows`) so the OS can grow the guard page on demand.
///
/// The UEFI scaffold has no demand-paged stack and no guard pages —
/// the firmware allocates a flat stack region that's already mapped.
/// The correct behavior for this environment is to do nothing and
/// return so the prologue continues. A future commit (post-#346d,
/// if/when we install our own page tables with guard regions) will
/// replace this with a real probe loop matching the aarch64 arm's
/// approach.
///
/// Calling convention note: LLVM-emitted `__chkstk` calls on ARM do
/// NOT follow the AAPCS — the call comes from the function prologue
/// before SP has been adjusted, and `r4` carries the desired frame
/// size. The MSVC implementation walks pages touching `[sp - r4 ..
/// sp]` to fault in the stack. Our scaffold has no demand paging,
/// so we just return — `bx lr` does the right thing.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __chkstk() {
    core::arch::naked_asm!("bx lr")
}
