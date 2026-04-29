// crates/arest-kernel/src/arch/aarch64/entropy.rs
//
// aarch64 hardware entropy source for the UEFI kernel (#570 / Rand-T2).
// Sibling of the x86_64 RDSEED/RDRAND adapter in
// `arch::uefi::x86_64::entropy` (#569). Same shape — feature-detect at
// construction, dispatch `fill()` through a 64-bit step function with a
// per-chunk retry budget, install once at boot via
// `arest::entropy::install`.
//
// Why two instructions
// --------------------
// ARMv8.5-A introduces a pair of system registers reachable via the
// `mrs` instruction:
//
//   * RNDR   — pulls 64 bits from an on-chip DRBG. NZCV.C=1 on success,
//              NZCV.C=0 on transient entropy starvation (retry).
//   * RNDRRS — same as RNDR, but conditioned to RESEED the DRBG before
//              producing the value. Slower per call (the reseed
//              consumes raw noise-source output) but stronger — what
//              you want for seeding another CSPRNG.
//
// We prefer RNDRRS for a CSPRNG seed (its output IS conditioned on
// fresh noise-source bytes, no purely-DRBG continuity between calls)
// and fall back to RNDR when the reseed variant repeatedly times out.
// On silicon without FEAT_RNG (every pre-ARMv8.5-A core, plus some
// vendors that mask the bits), `fill()` returns
// `EntropyError::HardwareUnavailable` and the boot path can chain to
// the UEFI `EFI_RNG_PROTOCOL` fallback (#571 / Rand-T3).
//
// Feature detection
// -----------------
// `ID_AA64ISAR0_EL1` bits [63:60] hold the RNDR feature field:
//   * 0b0000 — neither RNDR nor RNDRRS implemented (FEAT_RNG absent).
//   * 0b0001 — both RNDR and RNDRRS implemented.
//   * Higher values are reserved for forward-compat extensions.
//
// The probe runs once at construction; per-call cost is a plain match
// on a cached `Mode`.
//
// Retry budget
// ------------
// ARM's "Architecture Reference Manual for A-profile" (DDI 0487, §D7.5)
// notes that NZCV.C may be cleared when the on-chip DRBG cannot supply
// a fresh value within an implementation-defined window — same kind of
// transient that x86's RDSEED exhibits. We pick 100 retries per chunk
// to match the x86_64 arm; the cost is a few `yield` instructions per
// failed read, negligible against a successful 8-byte pull.
//
// Inline assembly
// ---------------
// `mrs` cannot be expressed via stable intrinsics — `core::arch::asm!`
// is the only path. We emit `mrs` for both the value read AND the
// NZCV read because the value-producing `mrs RNDR` clobbers NZCV
// itself (the spec says C is set/cleared as the success/fail flag),
// and `options(preserves_flags)` would be a lie. Reading NZCV via
// a second `mrs` lifts the flag bits into a normal GPR so the surrounding
// Rust code can branch on them without losing anything to the next
// instruction the optimiser inserts.

#[allow(unused_imports)]
use alloc::{boxed::Box, string::{String, ToString}, vec::Vec, format};

use arest::entropy::{EntropyError, EntropySource};

// ── Tunables ────────────────────────────────────────────────────────

/// Per-8-byte retry budget. RNDR/RNDRRS may transiently fail when the
/// on-chip DRBG hasn't reseeded yet; ARM doesn't publish a hard upper
/// bound but the failure rate is comparable to x86 RDSEED. We pick the
/// same 100 the x86_64 arm uses so the loop bodies match shape and the
/// behaviour is uniform across arches.
const PER_CHUNK_RETRIES: u32 = 100;

// ── Mode selection ──────────────────────────────────────────────────

/// Which hardware path the source dispatches to. Decided once at
/// construction via `ID_AA64ISAR0_EL1.RNDR`; `fill()` reads this
/// without re-probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// FEAT_RNG present — both RNDRRS and RNDR usable. The fill loop
    /// tries RNDR first per chunk (cheaper, DRBG-buffered) and falls
    /// back to RNDRRS when RNDR repeatedly returns NZCV.C=0.
    Rng,
    /// FEAT_RNG absent — `fill()` returns `HardwareUnavailable`. Boot
    /// path can chain to `EFI_RNG_PROTOCOL` fallback (#571).
    None,
}

// ── Public surface ──────────────────────────────────────────────────

/// aarch64 hardware entropy source. Detects FEAT_RNG via
/// `ID_AA64ISAR0_EL1` at construction and dispatches `fill()` through
/// the RNDR / RNDRRS pair. `Send + Sync` because the global slot in
/// `arest::entropy::GLOBAL_SOURCE` lives behind a spin lock and a
/// single source is shared across every CPU once SMP comes online.
pub struct Aarch64HwEntropy {
    mode: Mode,
}

impl Aarch64HwEntropy {
    /// Probe `ID_AA64ISAR0_EL1` for FEAT_RNG and build a source. The
    /// probe is constant-time (a single `mrs`), so calling this from
    /// boot has no measurable cost on hosts that mask the feature.
    pub fn new() -> Self {
        let mode = if cpuid_supports_rndr() { Mode::Rng } else { Mode::None };
        Self { mode }
    }

    /// Test-only escape hatch — build a source forced into a specific
    /// mode without re-probing. Lets the unit tests exercise the
    /// `None` branch on hosts that DO have FEAT_RNG, and walk the
    /// chunk-loop logic without depending on actual hardware.
    #[cfg(test)]
    fn with_mode(mode: Mode) -> Self {
        Self { mode }
    }
}

impl Default for Aarch64HwEntropy {
    fn default() -> Self {
        Self::new()
    }
}

impl EntropySource for Aarch64HwEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<usize, EntropyError> {
        match self.mode {
            Mode::Rng => fill_with_rng(buf),
            Mode::None => Err(EntropyError::HardwareUnavailable),
        }
    }
}

// ── FEAT_RNG probe ──────────────────────────────────────────────────

/// Returns `true` when `ID_AA64ISAR0_EL1.RNDR` (bits [63:60]) is non-
/// zero, meaning the silicon implements both RNDR and RNDRRS. Pure
/// `mrs` read of the system register — no side effects.
///
/// SAFETY: `mrs ID_AA64ISAR0_EL1` is unprivileged at EL1 and exists
/// on every aarch64 CPU since ARMv8.0 (the field encodes "implemented
/// since"; reading the register on a pre-FEAT_RNG core just returns
/// 0b0000 in the top nibble — no #UD analogue). UEFI on aarch64 always
/// hands control at EL1 or higher.
#[cfg(target_arch = "aarch64")]
fn cpuid_supports_rndr() -> bool {
    use core::arch::asm;
    let isar0: u64;
    // SAFETY: see function docstring. `nostack` + `preserves_flags`
    // are both accurate — `mrs` reading a feature ID register has no
    // memory or stack effects and doesn't touch NZCV.
    unsafe {
        asm!(
            "mrs {isar0}, ID_AA64ISAR0_EL1",
            isar0 = out(reg) isar0,
            options(nostack, preserves_flags),
        );
    }
    ((isar0 >> 60) & 0xF) >= 1
}

/// Host-target stub for the FEAT_RNG probe. Lives behind the
/// non-aarch64 `cfg` so the module compiles on the test host
/// (`x86_64-pc-windows-msvc`, etc.) where `mrs` would be a #UD. The
/// stub returns `false`, which routes `Aarch64HwEntropy::new()` into
/// `Mode::None` and lets the host-side tests exercise the
/// HardwareUnavailable branch directly.
#[cfg(not(target_arch = "aarch64"))]
fn cpuid_supports_rndr() -> bool {
    false
}

// ── Per-instruction wrappers ────────────────────────────────────────

/// Try a single 8-byte read from RNDR. Returns `Some(v)` on success
/// (NZCV.C=1) or `None` on transient failure (NZCV.C=0). Two `mrs`
/// instructions: one for the value, one to lift NZCV into a GPR for
/// the C-flag check.
///
/// SAFETY: `mrs RNDR` traps as #UD on pre-FEAT_RNG silicon. The caller
/// (`fill_with_rng`) is gated by `Mode::Rng`, set only after
/// `cpuid_supports_rndr()` returned `true`.
#[cfg(target_arch = "aarch64")]
fn try_rndr() -> Option<u64> {
    use core::arch::asm;
    let v: u64;
    let nzcv: u64;
    // SAFETY: see function docstring. We deliberately omit
    // `preserves_flags` — `mrs RNDR` writes NZCV.C to signal success,
    // so the asm DOES alter flags. The follow-up `mrs NZCV` reads
    // them; the `out(reg)` for `nzcv` must observe the value `mrs
    // RNDR` produced, hence the two reads sit in the same asm block
    // (no compiler-inserted instructions between them).
    unsafe {
        asm!(
            "mrs {v}, RNDR",
            "mrs {nzcv}, NZCV",
            v = out(reg) v,
            nzcv = out(reg) nzcv,
            options(nostack),
        );
    }
    // NZCV.C is bit 29. Set means valid; cleared means transient
    // failure (DRBG empty this cycle).
    if (nzcv >> 29) & 1 == 1 { Some(v) } else { None }
}

/// Try a single 8-byte read from RNDRRS — the reseed-then-read variant.
/// Slower than RNDR (the DRBG reseeds from raw noise-source output
/// before producing the value) but stronger; used as the fallback
/// when RNDR repeatedly returns None.
///
/// SAFETY: same gating as `try_rndr` — `Mode::Rng` is only entered
/// after FEAT_RNG was detected, and FEAT_RNG implies BOTH RNDR and
/// RNDRRS per ARM ARM §D7.5.
#[cfg(target_arch = "aarch64")]
fn try_rndrrs() -> Option<u64> {
    use core::arch::asm;
    let v: u64;
    let nzcv: u64;
    // SAFETY: see function docstring. NZCV is clobbered by `mrs
    // RNDRRS`; the second `mrs NZCV` lifts the flag bits into a GPR
    // for the success check.
    unsafe {
        asm!(
            "mrs {v}, RNDRRS",
            "mrs {nzcv}, NZCV",
            v = out(reg) v,
            nzcv = out(reg) nzcv,
            options(nostack),
        );
    }
    if (nzcv >> 29) & 1 == 1 { Some(v) } else { None }
}

// Host-target stubs. Returning `None` mirrors the behaviour of a
// FEAT_RNG-absent core (every read is a transient miss); since the
// host build can only reach these via `Mode::Rng` constructed by the
// test-only `with_mode` helper, the stubs exist purely to keep
// `cargo check` / `cargo test` green on non-aarch64 hosts.

#[cfg(not(target_arch = "aarch64"))]
fn try_rndr() -> Option<u64> {
    None
}

#[cfg(not(target_arch = "aarch64"))]
fn try_rndrrs() -> Option<u64> {
    None
}

// ── Fill loop ───────────────────────────────────────────────────────

/// Pull entropy via RNDR (with RNDRRS fallback) into `buf` until the
/// buffer is full or the per-chunk retry budget is exhausted. Returns
/// the number of bytes actually written; the caller
/// (`arest::entropy::fill`) loops on partial reads.
///
/// Each iteration reads 8 bytes (the natural width of RNDR/RNDRRS).
/// Tail bytes (when `buf.len() % 8 != 0`) are handled by reading one
/// more u64 and copying only the needed prefix.
///
/// Empty input is a no-op success — every fill source must accept a
/// zero-length buffer cleanly per the trait.
///
/// Per-chunk algorithm:
///   1. Try RNDR up to PER_CHUNK_RETRIES times. RNDR is cheaper
///      (DRBG-buffered) so we lean on it for the bulk path.
///   2. If RNDR exhausted its retries, try RNDRRS up to
///      PER_CHUNK_RETRIES times. The reseed variant is more
///      expensive but pulls from a freshly-conditioned DRBG pool.
///   3. If both fail, return what we wrote so far. The caller's
///      outer loop will reissue and either succeed on a later call
///      or trip its own retry cap.
fn fill_with_rng(buf: &mut [u8]) -> Result<usize, EntropyError> {
    let total = buf.len();
    if total == 0 {
        return Ok(0);
    }

    let mut written = 0;
    while written < total {
        // Phase 1: try RNDR (cheaper).
        let mut value: Option<u64> = None;
        for _ in 0..PER_CHUNK_RETRIES {
            if let Some(v) = try_rndr() {
                value = Some(v);
                break;
            }
            yield_cpu();
        }

        // Phase 2: fall back to RNDRRS (reseed-then-read) when RNDR
        // exhausted its budget. RNDRRS is the stronger primitive —
        // each call conditions the DRBG on fresh noise — so it's the
        // right escalation when the buffered RNDR pool keeps coming
        // up empty.
        if value.is_none() {
            for _ in 0..PER_CHUNK_RETRIES {
                if let Some(v) = try_rndrrs() {
                    value = Some(v);
                    break;
                }
                yield_cpu();
            }
        }

        let v = match value {
            Some(v) => v,
            None => {
                // Both RNDR and RNDRRS exhausted. Return what we
                // already wrote (which may be 0); the caller's outer
                // loop will reissue and either succeed on a later
                // attempt or trip its own retry cap. This matches the
                // x86_64 arm's behaviour on an exhausted RDSEED retry.
                return Ok(written);
            }
        };

        // Copy the 8-byte chunk into the output buffer. The tail
        // (fewer than 8 bytes remaining) is handled by clamping `n`
        // to the remaining length and discarding the high bytes.
        let bytes = v.to_le_bytes();
        let n = (total - written).min(8);
        buf[written..written + n].copy_from_slice(&bytes[..n]);
        written += n;
    }

    Ok(written)
}

/// CPU-yield hint for spin-wait between retries. `yield` is the aarch64
/// analogue of x86's `pause`: it tells the CPU the current loop is a
/// busy-wait and lets the pipeline drop to a lower-power state for a
/// few cycles before retrying. Cheaper than spinning at full IPC.
///
/// SAFETY: `yield` is a hint instruction, has no memory or stack
/// effects, and never traps. `nomem` / `nostack` / `preserves_flags`
/// describe it accurately.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn yield_cpu() {
    use core::arch::asm;
    unsafe {
        asm!("yield", options(nomem, nostack, preserves_flags));
    }
}

/// Host-target stub: a plain spin-loop hint via `core::hint::spin_loop`.
/// Reachable only when the test-only `with_mode(Mode::Rng)` constructor
/// is used on a non-aarch64 host; the hint costs nothing and keeps the
/// fill-loop body uniform across cfgs.
#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
fn yield_cpu() {
    core::hint::spin_loop();
}

// ── EFI_RNG_PROTOCOL fallback re-use (#571) ────────────────────────
//
// The aarch64 UEFI boot path needs the same firmware-seeded fallback
// the x86_64 arm gets for QEMU-on-bare-TCG hosts (FEAT_RNG masked, no
// silicon entropy). The `BootSeedEntropy` and `ChainedEntropy` types
// in `arch::uefi::x86_64::entropy` are arch-neutral — pure FNV-1a-64
// keystream over a 32-byte seed — but they live under the x86_64
// directory because that's where #571 first landed. Rather than
// move them now (would clash with the x86_64 entropy module the
// other agents are working on), we re-declare the same surface here
// as thin wrappers over the same 32-byte seed.
//
// `BootSeedEntropy` derives bytes by repeatedly hashing
// `(counter || seed)` with FNV-1a-64. The counter is atomic so two
// concurrent fills (post-SMP) see distinct streams. Not crypto-grade
// in isolation; the csprng above stretches it via ChaCha20.

/// Bootstrap-seed length. Same constant as the x86_64 arm's
/// `efi_rng::SEED_LEN` — 32 bytes is the ChaCha20 key width, which
/// is the largest seed any downstream consumer needs.
pub const SEED_LEN: usize = 32;

/// Bootstrap entropy source backed by a 32-byte seed captured from
/// `EFI_RNG_PROTOCOL` before ExitBootServices. Each `fill()` call
/// derives output by hashing `(counter || seed)` with FNV-1a-64 and
/// writing 8 bytes per round, advancing the counter monotonically so
/// the same seed never produces the same output twice.
pub struct BootSeedEntropy {
    seed: [u8; SEED_LEN],
    counter: core::sync::atomic::AtomicU64,
}

impl BootSeedEntropy {
    pub fn new(seed: [u8; SEED_LEN]) -> Self {
        Self { seed, counter: core::sync::atomic::AtomicU64::new(0) }
    }
}

impl EntropySource for BootSeedEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<usize, EntropyError> {
        let mut written = 0;
        while written < buf.len() {
            let ctr = self.counter.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            // FNV-1a-64. Cheap, no_std-clean, good-enough opacity for
            // bootstrap bytes derived from a firmware-random seed.
            let mut h: u64 = 0xcbf29ce484222325;
            for b in ctr.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            for b in self.seed.iter() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            let block = h.to_le_bytes();
            let take = core::cmp::min(8, buf.len() - written);
            buf[written..written + take].copy_from_slice(&block[..take]);
            written += take;
        }
        Ok(written)
    }
}

/// Composite source: try `Aarch64HwEntropy` first, fall back to the
/// firmware-seeded keystream when the hardware path faults. Mirror of
/// the x86_64 arm's `ChainedEntropy` shape.
///
/// When `fallback` is `None` (no EFI RNG seed captured pre-EBS), this
/// degrades to bare `Aarch64HwEntropy` and a hardware fault still
/// surfaces as `HardwareUnavailable`. That's the pre-#571 behaviour.
pub struct ChainedEntropy {
    primary: Aarch64HwEntropy,
    fallback: Option<BootSeedEntropy>,
}

impl ChainedEntropy {
    pub fn new(primary: Aarch64HwEntropy, fallback: Option<BootSeedEntropy>) -> Self {
        Self { primary, fallback }
    }
}

impl EntropySource for ChainedEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<usize, EntropyError> {
        match self.primary.fill(buf) {
            Ok(n) => Ok(n),
            Err(EntropyError::HardwareUnavailable) => {
                match &mut self.fallback {
                    Some(fb) => fb.fill(buf),
                    None => Err(EntropyError::HardwareUnavailable),
                }
            }
            Err(other) => Err(other),
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// FEAT_RNG detection must return some plausible answer on every
    /// host. On non-aarch64 hosts the stub always returns `false`; on
    /// aarch64 hosts the answer depends on silicon. We assert only
    /// that the call completes without faulting.
    #[test]
    fn detect_feat_rng_returns_a_bool() {
        let _ = cpuid_supports_rndr();
    }

    /// On a host without FEAT_RNG (every non-aarch64 test runner, plus
    /// most aarch64 dev boxes today), `Aarch64HwEntropy::new()` lands
    /// in `Mode::None` and `fill()` MUST report unavailability. Anything
    /// else would be a false positive — silently handing zeroed bytes
    /// is the exact failure mode this module exists to prevent.
    #[test]
    fn fill_with_mode_none_reports_unavailable() {
        let mut src = Aarch64HwEntropy::with_mode(Mode::None);
        let mut buf = [0u8; 16];
        assert_eq!(src.fill(&mut buf), Err(EntropyError::HardwareUnavailable));
    }

    /// Empty buffer is a no-op success on hardware modes. Trait
    /// requires implementations to handle zero-length cleanly.
    #[test]
    fn fill_empty_buffer_is_noop_for_rng_mode() {
        let mut src = Aarch64HwEntropy::with_mode(Mode::Rng);
        let mut buf: [u8; 0] = [];
        let n = src
            .fill(&mut buf)
            .expect("zero-length fill never fails for rng mode");
        assert_eq!(n, 0);
    }

    /// `Default::default()` should match `new()` — useful for callers
    /// that want a hardware source via the standard trait. Doesn't
    /// inspect the mode (which depends on the host CPU).
    #[test]
    fn default_constructs_via_new() {
        let _ = Aarch64HwEntropy::default();
    }

    /// `BootSeedEntropy::fill` writes the requested length to a
    /// non-byte-aligned tail (17 bytes) and advances its counter so
    /// the second call produces different output. Exercises the
    /// chunk-walk arithmetic without depending on hardware.
    #[test]
    fn boot_seed_fill_buffer_walks_chunks() {
        let seed = [0x42u8; SEED_LEN];
        let mut src = BootSeedEntropy::new(seed);
        let mut buf = [0u8; 17];
        let n = src.fill(&mut buf).expect("fnv keystream never faults");
        assert_eq!(n, 17, "all 17 bytes must be filled");
        // The keystream is not all-zero — FNV-1a-64 of a non-zero
        // seed plus a counter is overwhelmingly non-zero.
        assert!(buf.iter().any(|&b| b != 0), "keystream must be non-zero");

        // Second call advances the counter, producing different bytes.
        let mut buf2 = [0u8; 17];
        src.fill(&mut buf2).expect("fnv keystream never faults");
        assert_ne!(buf, buf2, "counter advancement must change output");
    }

    /// `ChainedEntropy` falls through to the BootSeedEntropy fallback
    /// when the primary reports HardwareUnavailable. With `Mode::None`
    /// forced into the primary AND a seed supplied to the fallback,
    /// the chained `fill()` must return the FNV-stretched keystream
    /// rather than HardwareUnavailable.
    #[test]
    fn chained_entropy_falls_through_to_boot_seed() {
        let primary = Aarch64HwEntropy::with_mode(Mode::None);
        let fallback = BootSeedEntropy::new([0x99u8; SEED_LEN]);
        let mut chained = ChainedEntropy::new(primary, Some(fallback));
        let mut buf = [0u8; 24];
        let n = chained.fill(&mut buf).expect("fallback should succeed");
        assert_eq!(n, 24);
        assert!(buf.iter().any(|&b| b != 0));
    }

    /// `ChainedEntropy` with no fallback degrades to bare primary —
    /// hardware fault surfaces as HardwareUnavailable.
    #[test]
    fn chained_entropy_no_fallback_propagates_unavailable() {
        let primary = Aarch64HwEntropy::with_mode(Mode::None);
        let mut chained = ChainedEntropy::new(primary, None);
        let mut buf = [0u8; 8];
        assert_eq!(chained.fill(&mut buf), Err(EntropyError::HardwareUnavailable));
    }

    /// On aarch64 hosts where FEAT_RNG IS present, `fill()` should
    /// succeed and produce non-zero bytes. On hosts without FEAT_RNG
    /// (or non-aarch64 hosts where the stub probe returns false), the
    /// constructor lands in `Mode::None` and we assert the
    /// HardwareUnavailable branch instead. Either way the trait
    /// contract is exercised end-to-end.
    #[test]
    fn rndr_supported_returns_value_or_reports_unavailable() {
        let mut src = Aarch64HwEntropy::new();
        let mut buf = [0u8; 32];
        match src.fill(&mut buf) {
            Ok(n) => {
                // On real FEAT_RNG silicon: we wrote some prefix and
                // the bytes are not all zero. n==0 would mean the
                // per-chunk retry budget exhausted on the very first
                // pull — vanishingly unlikely on real hardware.
                assert!(n > 0, "expected at least some bytes written");
                assert!(buf[..n].iter().any(|&b| b != 0), "must produce non-zero bytes");
            }
            Err(EntropyError::HardwareUnavailable) => {
                // Either non-aarch64 host or aarch64 silicon without
                // FEAT_RNG. Correct behaviour — the boot path chains
                // to the EFI_RNG_PROTOCOL fallback in this case.
            }
            Err(other) => panic!("unexpected error from rng fill: {other}"),
        }
    }
}
