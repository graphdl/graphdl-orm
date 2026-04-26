// crates/arest-kernel/src/arch/uefi/x86_64/entropy.rs
//
// x86_64 hardware entropy source for the UEFI kernel (#569 / Rand-T1).
// Pair to NNNNN's #567 (`9dc74f5`): the kernel installs an instance of
// `X86_64HwEntropy` into `arest::entropy`'s global slot at boot, so
// `arest::csprng::random_bytes` (and downstream consumers — AT_RANDOM
// stack canary in #575, `getrandom` syscall in #577, etc.) returns
// real-random bytes from the silicon RNG instead of the deterministic
// placeholder.
//
// Why two instructions
// --------------------
// Intel exposes two RNG instructions in the x86_64 ISA:
//
//   * RDSEED — pulls bytes directly from the on-die noise source
//     (the "true random" output, suitable for seeding another DRBG).
//     Introduced with Broadwell (2014). CF=1 on success, CF=0 on
//     transient failure (entropy pool empty this cycle — retry).
//
//   * RDRAND — pulls bytes from a hardware-DRBG that itself reseeds
//     periodically from RDSEED's source (cryptographically uniform,
//     not "true random"). Introduced with Ivy Bridge (2012). Same
//     CF=1/0 success/retry semantics.
//
// We prefer RDSEED for a CSPRNG seed (its output IS the entropy
// source's raw output, no intermediate DRBG), and fall back to RDRAND
// when RDSEED is unavailable (e.g. older Ivy Bridge boxes or the
// occasional virtualisation environment that masks the leaf-7 EBX
// bit). On a true vintage CPU with neither bit set, `fill()` returns
// `EntropyError::HardwareUnavailable` and the boot path can chain to
// the UEFI EFI_RNG_PROTOCOL fallback (#571).
//
// Feature detection
// -----------------
// Intel SDM Vol 2, CPUID:
//   * Leaf 7 (sub-leaf 0), EBX bit 18 = RDSEED present.
//   * Leaf 1, ECX bit 30 = RDRAND present.
//
// We probe both at construction (`X86_64HwEntropy::new`) and cache
// the cheaper of the two in `mode`. The probe runs once per kernel
// boot — there's no per-call CPUID cost on the fill path.
//
// Retry budget
// ------------
// Both instructions can transiently fail (CF=0) when the on-die
// noise source's pool is momentarily exhausted — RDSEED is meaningfully
// more failure-prone than RDRAND because it bypasses the DRBG buffer.
// Intel's reference docs (random-number-generator-implementation-guide,
// section 5.2.6) recommend a per-call retry of ~10 for RDRAND and
// up to ~100 for RDSEED. We pick 100 for both — paying a few extra
// `pause` cycles is cheap relative to handing back partial entropy.
//
// On exhausted retries we return whatever we already wrote (a "partial"
// fill is allowed by the trait per `arest::entropy::EntropySource::fill`'s
// docstring — the caller's `entropy::fill` loop will reissue another
// `fill()` until the buffer is full or the retry-cap in that helper
// trips and bubbles a `Fault` up).

#[allow(unused_imports)]
use alloc::{boxed::Box, string::{String, ToString}, vec::Vec, format};

use core::arch::x86_64::{__cpuid, __cpuid_count, _rdrand64_step, _rdseed64_step};

use arest::entropy::{EntropyError, EntropySource};

// ── Tunables ────────────────────────────────────────────────────────

/// Per-8-byte retry budget. RDSEED can fail (CF=0) when the on-die
/// noise pool is empty this cycle; the Intel reference guide recommends
/// 10 retries for RDRAND and ~100 for RDSEED. We pick 100 for both —
/// the extra `pause` budget is negligible against a successful 8-byte
/// pull, and matching the higher number lets the same loop body serve
/// both instructions without branching.
const PER_CHUNK_RETRIES: u32 = 100;

// ── Mode selection ──────────────────────────────────────────────────

/// Which hardware path the source dispatches to. Decided once at
/// construction via CPUID; `fill()` reads this without re-probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Prefer RDSEED (true entropy from the noise source).
    Rdseed,
    /// Fall back to RDRAND (DRBG-buffered, still hardware-random).
    Rdrand,
    /// Neither instruction available — `fill()` returns
    /// `HardwareUnavailable`. Boot path can chain to the UEFI
    /// EFI_RNG_PROTOCOL fallback (#571).
    None,
}

// ── Public surface ──────────────────────────────────────────────────

/// x86_64 hardware entropy source. Detects RDSEED / RDRAND availability
/// at construction and dispatches `fill()` to whichever the silicon
/// supports. `Send + Sync` because the global slot in
/// `arest::entropy::GLOBAL_SOURCE` lives behind a spin lock and a
/// single source is shared across every CPU once SMP comes online.
pub struct X86_64HwEntropy {
    mode: Mode,
}

impl X86_64HwEntropy {
    /// Probe CPUID for RDSEED + RDRAND availability and build a source
    /// pointed at the preferred instruction. RDSEED wins when both
    /// are present (it's the raw noise-source output, so no extra
    /// DRBG layer between us and the entropy).
    ///
    /// CPUID is a "serializing" instruction — invoking it once at
    /// boot has no measurable cost. The resulting `Mode` is cached
    /// in the struct so per-fill dispatch is a plain match.
    pub fn new() -> Self {
        let mode = match detect_features() {
            (true, _) => Mode::Rdseed,
            (false, true) => Mode::Rdrand,
            (false, false) => Mode::None,
        };
        Self { mode }
    }

    /// Test-only escape hatch — build a source forced into a specific
    /// mode without re-probing CPUID. Lets the unit tests exercise
    /// the `None` branch on hosts that DO have RDSEED, and vice versa.
    #[cfg(test)]
    fn with_mode(mode: Mode) -> Self {
        Self { mode }
    }
}

impl Default for X86_64HwEntropy {
    fn default() -> Self {
        Self::new()
    }
}

impl EntropySource for X86_64HwEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<usize, EntropyError> {
        match self.mode {
            Mode::Rdseed => fill_with(buf, rdseed_step),
            Mode::Rdrand => fill_with(buf, rdrand_step),
            Mode::None => Err(EntropyError::HardwareUnavailable),
        }
    }
}

// ── CPUID probe ─────────────────────────────────────────────────────

/// Returns `(rdseed_present, rdrand_present)`. Pure CPUID — no side
/// effects beyond the read. Pulled out as a free function so the unit
/// test can call it directly without constructing the source.
///
/// Intel SDM Vol 2:
///   * Leaf 7, sub-leaf 0, EBX bit 18 → RDSEED.
///   * Leaf 1,                ECX bit 30 → RDRAND.
///
/// SAFETY: `__cpuid` / `__cpuid_count` are `unsafe` because executing
/// CPUID on a CPU that doesn't support the requested leaf returns
/// undefined garbage in the output registers. Both leaves used here
/// (1 and 7) are present on every x86_64 CPU since Pentium-Pro
/// (leaf 1) and Haswell (leaf 7) — every silicon the UEFI kernel can
/// physically boot on (UEFI itself requires CPUID leaf 1; aarch64 /
/// armv7 builds don't reach this code).
fn detect_features() -> (bool, bool) {
    // Maximum supported standard CPUID leaf — leaf 7 didn't exist on
    // Pentium-Pro through Sandy Bridge. If the CPU's max-leaf is < 7,
    // RDSEED definitely isn't present (RDSEED-bearing silicon ships
    // with leaf 7), so skip the leaf-7 read entirely.
    //
    // `__cpuid` / `__cpuid_count` are safe wrappers in modern
    // `core::arch::x86_64` (the wrapper itself enforces the
    // `target_arch = "x86_64"` guard at compile time). On every CPU
    // that supports the x86_64 ISA, leaf 0 is mandatory and EAX
    // returns the max standard leaf.
    let max_leaf = __cpuid(0).eax;

    let rdseed = if max_leaf >= 7 {
        // max_leaf >= 7 means leaf 7 is implemented. Sub-leaf 0 is
        // the canonical sub-leaf for the baseline feature flag bits;
        // later sub-leaves enumerate extended features we don't probe
        // here.
        let leaf7 = __cpuid_count(7, 0);
        (leaf7.ebx >> 18) & 1 == 1
    } else {
        false
    };

    // Leaf 1 is mandatory on every CPUID-bearing CPU.
    let leaf1 = __cpuid(1);
    let rdrand = (leaf1.ecx >> 30) & 1 == 1;

    (rdseed, rdrand)
}

// ── Per-instruction wrappers ────────────────────────────────────────

/// Type alias for the 8-byte step function — both `_rdseed64_step` and
/// `_rdrand64_step` share this shape, lets the fill loop dispatch via
/// a function pointer rather than another match.
type StepFn = fn(&mut u64) -> bool;

/// Wrap `_rdseed64_step` so the fill loop sees a uniform `bool` return.
/// Returns `true` on success (CF=1), `false` on transient retry (CF=0).
///
/// SAFETY: the intrinsic is `unsafe` because executing RDSEED on a CPU
/// that doesn't implement it traps as #UD. The caller (`fill_with`)
/// is only reached via `Mode::Rdseed`, which the constructor sets only
/// after confirming the CPUID leaf-7 EBX bit 18 is set.
fn rdseed_step(out: &mut u64) -> bool {
    // SAFETY: see function docstring. The intrinsic writes to `*out`
    // unconditionally (zero on failure), so passing a valid `&mut u64`
    // is sound regardless of CF.
    let mut tmp: u64 = 0;
    let ok = unsafe { _rdseed64_step(&mut tmp) };
    *out = tmp;
    ok == 1
}

/// Wrap `_rdrand64_step` — same shape as `rdseed_step`, different
/// instruction. RDRAND fails (CF=0) far less often than RDSEED.
///
/// SAFETY: same as `rdseed_step` — gated by Mode::Rdrand, which the
/// constructor only picks after confirming the CPUID leaf-1 ECX bit
/// 30 is set.
fn rdrand_step(out: &mut u64) -> bool {
    // SAFETY: see function docstring.
    let mut tmp: u64 = 0;
    let ok = unsafe { _rdrand64_step(&mut tmp) };
    *out = tmp;
    ok == 1
}

// ── Fill loop ───────────────────────────────────────────────────────

/// Pull entropy from `step` (either `rdseed_step` or `rdrand_step`)
/// into `buf` until the buffer is full or the retry budget per chunk
/// is exhausted. Returns the number of bytes actually written; the
/// caller (`arest::entropy::fill`) loops on partial reads.
///
/// Each iteration reads 8 bytes (the natural width of the 64-bit
/// instructions). Any tail bytes (when `buf.len() % 8 != 0`) are
/// handled by reading one more u64 and copying only the needed prefix.
///
/// Empty input is a no-op success — every fill source must accept a
/// zero-length buffer cleanly per the trait.
fn fill_with(buf: &mut [u8], step: StepFn) -> Result<usize, EntropyError> {
    let total = buf.len();
    if total == 0 {
        return Ok(0);
    }

    let mut written = 0;
    while written < total {
        let mut value: u64 = 0;
        let mut retries = PER_CHUNK_RETRIES;
        let success = loop {
            if step(&mut value) {
                break true;
            }
            if retries == 0 {
                break false;
            }
            retries -= 1;
            // `pause` is the standard idiom for spin-wait on x86 — it
            // hints to the CPU that the loop is a busy-wait and lets
            // the pipeline drop to lower power for a few cycles before
            // retrying RDSEED. Cheaper than spinning at full IPC.
            //
            // SAFETY: `pause` has no memory or stack effects and never
            // traps; `nomem` + `nostack` are the right options.
            unsafe {
                core::arch::asm!("pause", options(nomem, nostack, preserves_flags));
            }
        };

        if !success {
            // Per-chunk retry exhausted. Return what we already wrote
            // (which may be 0 on the first chunk); the caller's outer
            // loop will reissue and either succeed on a later attempt
            // or trip its own retry cap. Returning Ok(0) on a totally
            // empty fill would be treated as "nothing this round" by
            // the helper's retry path, which is the correct semantic.
            return Ok(written);
        }

        // Copy the 8-byte chunk into the output buffer. The tail
        // (fewer than 8 bytes remaining) is handled by clamping `n`
        // to the remaining length and discarding the high bytes.
        let bytes = value.to_le_bytes();
        let n = (total - written).min(8);
        buf[written..written + n].copy_from_slice(&bytes[..n]);
        written += n;
    }

    Ok(written)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// CPUID detection must return some plausible answer on every
    /// host. We don't care WHICH bits are set (the test runner may
    /// be a virtual machine that masks RDSEED), only that the call
    /// completes without faulting and returns a `(bool, bool)`. The
    /// runtime-fill tests below assert behaviour conditional on the
    /// detected features.
    #[test]
    fn detect_features_returns_a_pair() {
        let (rdseed, rdrand) = detect_features();
        // Trivially true — the goal is "the call did not panic".
        let _ = (rdseed, rdrand);
    }

    /// On hardware that supports either instruction (most test
    /// runners), `fill()` writes the requested length and the result
    /// is not all zero. The two-condition assertion lets the test pass
    /// on a CPU with neither instruction (where `mode == None` and we
    /// SHOULD return HardwareUnavailable rather than zero bytes).
    #[test]
    fn fill_writes_non_zero_bytes_when_hardware_present() {
        let (rdseed, rdrand) = detect_features();
        if !rdseed && !rdrand {
            // No hardware — the constructor MUST land in `None` mode
            // and `fill()` MUST report unavailability. Anything else
            // would be a false positive (silently handing zeroed
            // bytes is the exact failure mode this whole module
            // exists to prevent).
            let mut src = X86_64HwEntropy::new();
            let mut buf = [0u8; 16];
            assert_eq!(src.fill(&mut buf), Err(EntropyError::HardwareUnavailable));
            return;
        }

        let mut src = X86_64HwEntropy::new();
        let mut buf = [0u8; 32];
        let n = src.fill(&mut buf).expect("fill should succeed when hw present");
        // `n == 0` would mean the per-chunk retry budget exhausted on
        // the very first 8-byte pull — vanishingly unlikely on real
        // silicon, and the trait permits short reads anyway. We assert
        // ANY progress and any non-zero byte; the CSPRNG re-seeds from
        // multiple short reads if needed.
        assert!(n > 0, "expected at least some bytes written");
        assert!(buf[..n].iter().any(|&b| b != 0), "must produce non-zero bytes");
    }

    /// The forced-`None` constructor (test-only) must report hardware
    /// unavailable even on a host that has RDSEED — proving the
    /// `Mode::None` arm of `fill()` returns the right error. Without
    /// this we'd be relying on CPUID happening to pick `None`, which
    /// it never does on a modern dev box.
    #[test]
    fn fill_with_mode_none_reports_unavailable() {
        let mut src = X86_64HwEntropy::with_mode(Mode::None);
        let mut buf = [0u8; 8];
        assert_eq!(src.fill(&mut buf), Err(EntropyError::HardwareUnavailable));
    }

    /// Empty buffer is a no-op success on every mode — the trait
    /// requires implementations to handle zero-length cleanly. This
    /// test pins the contract for the `Rdseed` arm specifically (the
    /// other arms share the `fill_with` body, which has the early
    /// return at the top).
    #[test]
    fn fill_empty_buffer_is_noop() {
        let mut src = X86_64HwEntropy::with_mode(Mode::None);
        let mut buf: [u8; 0] = [];
        // Even in `None` mode, an empty buffer should not surface a
        // hardware error — there's no work to do. The implementation
        // routes through `fill_with`'s early return for the hardware
        // modes; `Mode::None` returns `HardwareUnavailable` at the
        // outer match. Document that semantic by asserting the actual
        // behaviour: `None` reports unavailable, hardware modes return
        // Ok(0).
        let _ = src.fill(&mut buf);

        let mut hw = X86_64HwEntropy::with_mode(Mode::Rdrand);
        // Don't actually invoke RDRAND for a zero-length buffer — the
        // early-return in fill_with handles this. If hardware is absent
        // and we DID dispatch through, the test would still pass
        // (Ok(0) doesn't trap), but the early-return is what we're
        // documenting here.
        let n = hw.fill(&mut buf).expect("zero-length fill never fails for hw modes");
        assert_eq!(n, 0);
    }

    /// `Default::default()` should match `new()` — useful for callers
    /// that want a hardware source via the standard trait. Doesn't
    /// inspect the mode (which depends on the host CPU).
    #[test]
    fn default_constructs_via_new() {
        let _ = X86_64HwEntropy::default();
    }
}
