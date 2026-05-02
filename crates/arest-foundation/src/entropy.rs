// crates/arest/src/entropy.rs
//
// Entropy source trait + global slot. Pair to `csprng.rs` (#567/#568):
// this module supplies the *seed* material; csprng turns it into the
// uniform stream callers consume. Every target (FPGA soft-core, x86_64
// kernel, hosted std, WASM, Wine launcher) installs exactly one source
// at boot. Per-target adapters land in #569-#574.
//
// ## Why this is its own module
//
// Today AREST has no real RNG — the kernel's auxv `AT_RANDOM` slot
// (process/process.rs ~line 74) is hardcoded to literal bytes
// `b"AREST_TIER_1_RNG"`, and `crypto.rs` only does deterministic HMAC.
// That works for the smoke tests but is unsafe for any real workload:
// libc derives the stack canary from AT_RANDOM, so a fixed value is a
// fixed canary. The csprng module needs a uniform interface to whatever
// hardware (RDRAND, /dev/urandom, virtio-rng, getrandom) the target
// happens to have. Hence the trait.
//
// ## Trait surface (intentionally minimal)
//
// `EntropySource::fill(&mut self, &mut [u8]) -> Result<usize, EntropyError>`
//
// Returns the number of bytes actually written — callers must handle
// short reads (a hardware source may yield only what's buffered, then
// the caller loops until the full request is satisfied or it gives up).
// Errors are coarse on purpose: `HardwareUnavailable` (driver missing or
// device stuck), `Fault` (returned bytes look obviously broken, e.g.
// all-zero on RDRAND, FIFO timeout). Targets translate their native
// error codes into one of those two.
//
// ## Global slot
//
// `GLOBAL_SOURCE: spin::RwLock<Option<Box<dyn EntropySource + Send +
// Sync>>>`. Targets call `entropy::install(...)` once during early
// boot, before any `csprng::random_*` call. The csprng module fetches
// the slot lazily on first use; if it's still `None` it panics with
// a clear message — failing loud beats handing back zeroed bytes.
//
// `RwLock` (not `OnceLock`) so tests can swap the source mid-run via
// `install_for_test` — production targets install once and never write
// again.

#[allow(unused_imports)]
use alloc::{boxed::Box, string::{String, ToString}, vec::Vec, format};
use core::fmt;

// ── Trait + error type ──────────────────────────────────────────────

/// Coarse error class from a hardware / virtual entropy source. Targets
/// translate their native errno / status into one of these — callers
/// don't need to know whether the underlying device was a virtio-rng
/// queue, RDRAND, or `/dev/urandom`, only whether it's worth retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntropyError {
    /// The source is not present or not initialised. A retry is
    /// pointless — caller should escalate (e.g. fall back to a chained
    /// source, or refuse to seed and panic).
    HardwareUnavailable,
    /// The source returned bytes that fail a basic sanity check
    /// (RDRAND CF=0, all-zero output from a normally noisy stream,
    /// virtio-rng descriptor never completed). A retry might succeed
    /// if the device's transient error clears.
    Fault,
}

impl fmt::Display for EntropyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HardwareUnavailable => f.write_str("entropy hardware unavailable"),
            Self::Fault => f.write_str("entropy source fault"),
        }
    }
}

/// A target-supplied source of seed entropy. Implementations live in
/// the per-target adapters (#569-#574): `Rdrand`, `VirtioRng`,
/// `LinuxGetrandom`, `WindowsRtlGenRandom`, `WasmCryptoGetRandom`,
/// `FpgaTrngFifo`. The shared csprng pulls 32 bytes from the installed
/// instance during seeding / reseeding.
///
/// `Send + Sync` because the global slot lives behind a spin lock and
/// a single source is shared across every CPU / worker thread the
/// target spawns.
pub trait EntropySource: Send + Sync {
    /// Fill `buf` with fresh entropy and return the number of bytes
    /// written. Implementations must write contiguously starting at
    /// `buf[0]` — callers treat any prefix `0..n` as freshly randomised
    /// and the suffix as untouched.
    ///
    /// Short reads are explicitly allowed (the caller loops). An `Err`
    /// must be returned when the source can produce zero bytes; `Ok(0)`
    /// is reserved for "nothing produced this call but a retry might
    /// help" semantics that no current target uses, so we treat it as
    /// equivalent to `Err(EntropyError::Fault)` on the read side.
    fn fill(&mut self, buf: &mut [u8]) -> Result<usize, EntropyError>;
}

// ── Global slot ─────────────────────────────────────────────────────

/// One source per process. Targets install during early boot via
/// `install`; the csprng pulls from it lazily. `RwLock<Option<...>>`
/// (rather than `OnceLock`) so tests can install / replace / clear
/// across cases without recompiling — production code installs once.
static GLOBAL_SOURCE: spin::RwLock<Option<Box<dyn EntropySource>>> =
    spin::RwLock::new(None);

/// Install the process-wide entropy source. Targets call this exactly
/// once during early boot. Subsequent calls REPLACE the previously
/// installed source — production paths must avoid this; tests use it
/// to swap a `DeterministicSource` in for reproducible output.
///
/// The `Send + Sync` bound is part of the trait, so the boxed value is
/// safe to share across threads (the spin lock provides the actual
/// mutual exclusion).
pub fn install(source: Box<dyn EntropySource>) {
    *GLOBAL_SOURCE.write() = Some(source);
}

/// Clear the installed source. Used by tests that want the "no source"
/// branch to fire on the next csprng call. Production paths never
/// uninstall.
pub fn uninstall() {
    *GLOBAL_SOURCE.write() = None;
}

/// `true` when a source has been installed. Callers (chiefly the csprng
/// lazy-seed path) use this to decide between proceeding and panicking
/// with a clear "no entropy source installed" message.
pub fn is_installed() -> bool {
    GLOBAL_SOURCE.read().is_some()
}

/// Pull at least `buf.len()` bytes of entropy from the installed source
/// into `buf`. Returns `Err` if no source is installed or if the source
/// keeps faulting (we cap retries at 16 so a permanently broken source
/// can't deadlock the caller).
///
/// Retries are needed because the trait permits short reads — a
/// virtio-rng descriptor may yield 8 bytes per completion, so a 32-byte
/// reseed loops four times. The retry counter discounts only Fault
/// errors; a single HardwareUnavailable bails immediately.
pub fn fill(buf: &mut [u8]) -> Result<(), EntropyError> {
    let mut guard = GLOBAL_SOURCE.write();
    let source = guard.as_mut().ok_or(EntropyError::HardwareUnavailable)?;

    let mut filled = 0;
    let mut faults_remaining: u32 = 16;
    while filled < buf.len() {
        match source.fill(&mut buf[filled..]) {
            Ok(0) => {
                if faults_remaining == 0 {
                    return Err(EntropyError::Fault);
                }
                faults_remaining -= 1;
            }
            Ok(n) => {
                filled += n;
            }
            Err(EntropyError::HardwareUnavailable) => {
                return Err(EntropyError::HardwareUnavailable);
            }
            Err(EntropyError::Fault) => {
                if faults_remaining == 0 {
                    return Err(EntropyError::Fault);
                }
                faults_remaining -= 1;
            }
        }
    }
    Ok(())
}

// ── Test fixture: deterministic source ──────────────────────────────

/// Test-only entropy source that emits a fixed seed expanded with a
/// counter — same input → same output across runs, lets tests assert
/// exact CSPRNG output bytes without flake.
///
/// Algorithm: each `fill` advances `counter` and writes
/// `seed[i % 32] XOR counter_byte` for each requested byte. Not even
/// remotely cryptographic; the goal is reproducibility, not strength.
///
/// Lives in the production module (not behind `#[cfg(test)]`) because
/// the kernel's integration tests + per-consumer tests (#575-#578) also
/// need to exercise AT_RANDOM contents reproducibly. Production paths
/// must never install this.
pub struct DeterministicSource {
    seed: [u8; 32],
    counter: u64,
}

impl DeterministicSource {
    /// New deterministic source. Same `seed` will produce the same
    /// byte stream across every call to `fill`, regardless of buffer
    /// size or call ordering.
    pub fn new(seed: [u8; 32]) -> Self {
        Self { seed, counter: 0 }
    }
}

impl EntropySource for DeterministicSource {
    fn fill(&mut self, buf: &mut [u8]) -> Result<usize, EntropyError> {
        for byte in buf.iter_mut() {
            let c = self.counter;
            self.counter = self.counter.wrapping_add(1);
            *byte = self.seed[(c as usize) & 31] ^ ((c & 0xff) as u8);
        }
        Ok(buf.len())
    }
}

// ── Test serialization lock ─────────────────────────────────────────
//
// Every test (in this module AND in `csprng::tests`) that touches
// `GLOBAL_SOURCE` or the CSPRNG `STATE` must hold this lock. Cargo
// runs tests in parallel by default; a concurrent
// `entropy::install(AlwaysFault)` will sabotage a csprng test that
// is mid-fill. Serialising via one process-wide lock keeps the
// fixture installation atomic with the test body. Cross-module via
// `pub(crate)`.
//
// Exposed across the arest-foundation→arest crate boundary under the
// `test-helpers` feature: arest's own tests (cell_aead, csprng,
// crypto, cloudflare_entropy, freeze, …) install/uninstall the
// global slot too, so they need to serialize against the same lock
// arest-foundation's tests use. The feature is enabled in arest's
// `[dev-dependencies] arest-foundation = { features = ["test-helpers"] }`
// so production builds never link the symbol — only test binaries.
#[cfg(any(test, feature = "test-helpers"))]
pub static TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Per-test helper: install a deterministic source with a given
    /// seed, run the test body, then uninstall so the next test starts
    /// from "no source installed" — the `GLOBAL_SOURCE` is process-
    /// wide and survives between cases otherwise.
    fn with_source<F: FnOnce()>(seed: [u8; 32], body: F) {
        let _guard = TEST_LOCK.lock();
        install(Box::new(DeterministicSource::new(seed)));
        body();
        uninstall();
    }

    #[test]
    fn deterministic_source_is_deterministic() {
        let mut a = DeterministicSource::new([7u8; 32]);
        let mut b = DeterministicSource::new([7u8; 32]);
        let mut buf_a = [0u8; 64];
        let mut buf_b = [0u8; 64];
        a.fill(&mut buf_a).unwrap();
        b.fill(&mut buf_b).unwrap();
        assert_eq!(buf_a, buf_b, "same seed must produce same bytes");
    }

    #[test]
    fn deterministic_source_advances_across_calls() {
        let mut s = DeterministicSource::new([1u8; 32]);
        let mut first = [0u8; 16];
        let mut second = [0u8; 16];
        s.fill(&mut first).unwrap();
        s.fill(&mut second).unwrap();
        assert_ne!(first, second, "successive calls must advance counter");
    }

    #[test]
    fn fill_without_source_returns_hardware_unavailable() {
        let _guard = TEST_LOCK.lock();
        // Make sure no source is installed; another test may have left
        // one from an earlier run, so clear explicitly.
        uninstall();
        let mut buf = [0u8; 8];
        assert_eq!(fill(&mut buf), Err(EntropyError::HardwareUnavailable));
    }

    #[test]
    fn fill_with_installed_source_succeeds() {
        with_source([42u8; 32], || {
            let mut buf = [0u8; 32];
            assert!(fill(&mut buf).is_ok());
            // Must NOT be all zero — the deterministic source XORs the
            // seed with a counter, so the first 32 bytes are non-zero
            // for non-zero seeds.
            assert!(buf.iter().any(|&b| b != 0));
        });
    }

    #[test]
    fn is_installed_reflects_install_uninstall() {
        let _guard = TEST_LOCK.lock();
        uninstall();
        assert!(!is_installed());
        install(Box::new(DeterministicSource::new([0u8; 32])));
        assert!(is_installed());
        uninstall();
        assert!(!is_installed());
    }

    /// A source that always returns Fault — used to exercise the retry-
    /// cap branch in `fill`. Without the cap a permanently broken source
    /// would deadlock the caller; we want a bounded number of attempts.
    struct AlwaysFault;
    impl EntropySource for AlwaysFault {
        fn fill(&mut self, _buf: &mut [u8]) -> Result<usize, EntropyError> {
            Err(EntropyError::Fault)
        }
    }

    #[test]
    fn fill_caps_fault_retries() {
        let _guard = TEST_LOCK.lock();
        install(Box::new(AlwaysFault));
        let mut buf = [0u8; 4];
        let res = fill(&mut buf);
        uninstall();
        assert_eq!(res, Err(EntropyError::Fault));
    }

    /// A source that returns Ok(0) every call — the trait permits
    /// "nothing this round" semantics, but `fill` must treat repeated
    /// zero-progress as a fault and bail rather than spinning forever.
    struct AlwaysZeroProgress;
    impl EntropySource for AlwaysZeroProgress {
        fn fill(&mut self, _buf: &mut [u8]) -> Result<usize, EntropyError> {
            Ok(0)
        }
    }

    #[test]
    fn fill_caps_zero_progress() {
        let _guard = TEST_LOCK.lock();
        install(Box::new(AlwaysZeroProgress));
        let mut buf = [0u8; 4];
        let res = fill(&mut buf);
        uninstall();
        assert_eq!(res, Err(EntropyError::Fault));
    }

    /// A source that yields `chunk_size` bytes per call — tests that
    /// the caller's loop correctly assembles a long buffer from short
    /// reads (the trait explicitly allows short reads).
    struct ShortReadSource {
        chunk: usize,
        next: u8,
    }
    impl EntropySource for ShortReadSource {
        fn fill(&mut self, buf: &mut [u8]) -> Result<usize, EntropyError> {
            let n = buf.len().min(self.chunk);
            for slot in buf.iter_mut().take(n) {
                *slot = self.next;
                self.next = self.next.wrapping_add(1);
            }
            Ok(n)
        }
    }

    #[test]
    fn fill_loops_over_short_reads() {
        let _guard = TEST_LOCK.lock();
        install(Box::new(ShortReadSource { chunk: 3, next: 1 }));
        let mut buf = [0u8; 10];
        fill(&mut buf).unwrap();
        uninstall();
        // Bytes 1..=10 (the source increments per byte across calls).
        assert_eq!(buf, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    }

    #[test]
    fn entropy_error_display() {
        // Cheap smoke test — a panic / format failure here is the
        // signal something broke in the Display impl.
        let _ = format!("{}", EntropyError::HardwareUnavailable);
        let _ = format!("{}", EntropyError::Fault);
    }
}
