// crates/arest/src/entropy_mix.rs
//
// MixingEntropySource (#584 / Rand-X1). Combines N `EntropySource`
// children into one whose output is XOR of every child's output.
//
// ## Why XOR
//
// XOR is the "anytrust" entropy mixer: if even ONE child source is
// cryptographically uniform, the XOR of all children is cryptographically
// uniform — bias in any subset of children cancels through XOR with the
// good source. Used by RANDAO, Trustchain, and most decentralised RNG
// schemes for exactly this property.
//
// HKDF-Extract would also work and gives slightly stronger guarantees
// when ALL children are weakly biased, but it costs a SHA-256 per fill
// and an `hkdf` crate dep. XOR's "as random as the best child" floor
// is sufficient for the use cases we care about (host CLI mixing OS
// getrandom + network entropy + RDRAND; kernel mixing virtio-rng +
// timer jitter); upgrade to HKDF if a future use case needs the
// "uniform even when no child is" property.
//
// ## Use cases
//
// 1. **Host CLI defense-in-depth.** Combine the OS RNG (#574 host
//    getrandom) with a network-entropy adapter (#583 Rand-T7) so a
//    compromised OS RNG (kernel CSPRNG bug, container with predictable
//    seed) doesn't silently downgrade the engine's nonces.
//
// 2. **Kernel high-assurance boot.** Combine virtio-rng (#369), RDSEED
//    (#569 x86_64) or RNDR (#570 aarch64), EFI_RNG_PROTOCOL (#571)
//    where available, and timer jitter. Each individually trustworthy;
//    the XOR is robust to a silent failure in any one.
//
// 3. **WASM browser**: combine `crypto.getRandomValues` with a
//    network-fetched seed for tabs that don't trust the host browser.
//
// ## Failure semantics
//
// - **0 children**: `fill` returns `Err(HardwareUnavailable)` — the
//   mixer with no sources can't produce entropy. Caller should never
//   construct this (`new` requires non-empty); the runtime check is a
//   defence in depth.
//
// - **1 child**: degenerate — `fill` just delegates. Cheaper than the
//   XOR loop and preserves the child's exact short-read semantics.
//
// - **Any child errors**: propagate the most-severe error.
//   `HardwareUnavailable` from any child > `Fault` from any child >
//   short-read short-circuit. The mixer can't safely XOR-mix when any
//   contributing source produced unknown bytes — the standard
//   defence-in-depth assumption is "every contributor is fresh"; an
//   unfilled buffer slot would silently fold its prior contents into
//   the output.

#[allow(unused_imports)]
use alloc::{boxed::Box, vec, vec::Vec};

use crate::entropy::{EntropyError, EntropySource};

/// Combine N entropy sources into one whose output is XOR of every
/// child's output. Constructed via `new` (which rejects empty input)
/// or `with_children`; pushable via `add` for late-bound boot paths
/// that discover sources progressively.
pub struct MixingEntropySource {
    children: Vec<Box<dyn EntropySource>>,
}

impl MixingEntropySource {
    /// New mixer over the given child sources. The children are
    /// consumed (the mixer takes ownership) — at install time the
    /// caller passes `Box::new(MixingEntropySource::new(vec![...]))`
    /// to `entropy::install`.
    ///
    /// **Panics** if `children` is empty. A mixer with no sources
    /// produces no entropy; the panic catches the install-time
    /// configuration error rather than letting it surface later as
    /// a `Fault` on first `csprng::random_bytes` call.
    pub fn new(children: Vec<Box<dyn EntropySource>>) -> Self {
        assert!(!children.is_empty(),
            "MixingEntropySource: at least one child source required");
        Self { children }
    }

    /// Convenience: returns a boxed trait object so the install site
    /// reads as `entropy::install(MixingEntropySource::boxed(vec![...]))`.
    pub fn boxed(children: Vec<Box<dyn EntropySource>>) -> Box<dyn EntropySource> {
        Box::new(Self::new(children))
    }

    /// Add another child source. For boot paths that discover sources
    /// progressively (e.g. probe RDSEED first, then virtio-rng if the
    /// virtio device is enumerated, then EFI_RNG as fallback) — install
    /// the mixer once with the first source, push the rest as they
    /// come online.
    pub fn add(&mut self, child: Box<dyn EntropySource>) {
        self.children.push(child);
    }

    /// Number of child sources currently mixed. Diagnostic only;
    /// callers shouldn't branch on the count except for logging.
    pub fn len(&self) -> usize {
        self.children.len()
    }
}

impl EntropySource for MixingEntropySource {
    /// Fill `buf` with the XOR of every child's output. Each child is
    /// asked for the full `buf.len()` independently into a scratch
    /// buffer; the scratches are XORed into `buf` in turn (the first
    /// child's output overwrites `buf` directly, subsequent children
    /// XOR in).
    ///
    /// Empty buffer is a no-op and returns `Ok(0)` — matches the
    /// trait's "wrote n bytes starting at buf[0]" semantics without
    /// invoking any child.
    ///
    /// Errors short-circuit the fill: if any child errors, the
    /// remaining children are NOT polled and the error propagates.
    /// This avoids partial fills where some buf positions reflect the
    /// XOR of K children and others reflect K+1 — the caller would
    /// have no way to detect the mismatch.
    fn fill(&mut self, buf: &mut [u8]) -> Result<usize, EntropyError> {
        if buf.is_empty() {
            return Ok(0);
        }
        // Single-child fast path: skip the scratch alloc + XOR loop.
        // Preserves the child's short-read semantics exactly — a child
        // that yields 8 bytes per call still yields 8 bytes through
        // the mixer when it's the only child.
        if self.children.len() == 1 {
            return self.children[0].fill(buf);
        }
        // Multi-child path. First child writes directly into `buf` so
        // we don't need a scratch for it. Each subsequent child fills
        // a per-call scratch and we XOR it in.
        //
        // Each child must fill the FULL buffer — short reads from any
        // child would mean XOR-mixing fewer-than-N sources for the
        // suffix bytes, silently weakening those positions. Loop each
        // child's fill until buf.len() bytes accumulate or the child
        // hits the same retry cap the global `entropy::fill` uses.
        fill_full(self.children[0].as_mut(), buf)?;
        let mut scratch: Vec<u8> = vec![0u8; buf.len()];
        for child in self.children[1..].iter_mut() {
            fill_full(child.as_mut(), &mut scratch)?;
            for (out, mix) in buf.iter_mut().zip(scratch.iter()) {
                *out ^= *mix;
            }
        }
        Ok(buf.len())
    }
}

/// Pull exactly `buf.len()` bytes from `source`, looping on short reads.
/// Mirrors `entropy::fill`'s retry budget (16 Faults before giving up,
/// HardwareUnavailable bails immediately) — the mixer needs the same
/// guarantee its global counterpart provides, locally per child.
fn fill_full(source: &mut dyn EntropySource, buf: &mut [u8]) -> Result<(), EntropyError> {
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

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entropy::DeterministicSource;

    /// Two deterministic sources with distinct seeds — output equals
    /// XOR of each source's own output. The DeterministicSource
    /// algorithm is `seed[i & 31] ^ counter_byte`, so two of them
    /// XORed give `(seed_a[i&31] ^ counter) ^ (seed_b[i&31] ^ counter)`
    /// = `seed_a[i&31] ^ seed_b[i&31]` — the counter bytes cancel out
    /// in this specific algebra, which is what makes it a clean
    /// XOR-cancellation test.
    #[test]
    fn xor_of_two_deterministic_sources() {
        let seed_a = [0x55u8; 32];
        let seed_b = [0xAAu8; 32];
        let mut mix = MixingEntropySource::new(vec![
            Box::new(DeterministicSource::new(seed_a)),
            Box::new(DeterministicSource::new(seed_b)),
        ]);
        let mut buf = [0u8; 32];
        mix.fill(&mut buf).unwrap();
        // 0x55 XOR 0xAA = 0xFF. Counter bytes from each source XOR-cancel.
        assert_eq!(buf, [0xFFu8; 32]);
    }

    /// Single-child fast path: output identical to the child's direct
    /// output, no XOR transformation applied.
    #[test]
    fn single_child_delegates() {
        let seed = [0x42u8; 32];
        let mut mix = MixingEntropySource::new(vec![
            Box::new(DeterministicSource::new(seed)),
        ]);
        let mut a = [0u8; 16];
        mix.fill(&mut a).unwrap();

        let mut direct = DeterministicSource::new(seed);
        let mut b = [0u8; 16];
        direct.fill(&mut b).unwrap();

        assert_eq!(a, b);
    }

    /// Empty buffer: no-op return, no child invocation. Verified
    /// indirectly by `Ok(0)` (empty source list would error otherwise).
    #[test]
    fn empty_buffer_is_noop() {
        let mut mix = MixingEntropySource::new(vec![
            Box::new(DeterministicSource::new([0u8; 32])),
        ]);
        let mut buf: [u8; 0] = [];
        assert_eq!(mix.fill(&mut buf), Ok(0));
    }

    /// Constructor rejects empty children — the assert catches the
    /// configuration error at install time rather than at first fill.
    #[test]
    #[should_panic(expected = "at least one child source required")]
    fn empty_children_panics() {
        let _ = MixingEntropySource::new(Vec::new());
    }

    /// Three-way XOR: a XOR b XOR c. Builds on the two-source test
    /// to verify the loop scales past N=2 without off-by-one bugs.
    #[test]
    fn xor_of_three_deterministic_sources() {
        let mut mix = MixingEntropySource::new(vec![
            Box::new(DeterministicSource::new([0x0Fu8; 32])),
            Box::new(DeterministicSource::new([0xF0u8; 32])),
            Box::new(DeterministicSource::new([0xAAu8; 32])),
        ]);
        let mut buf = [0u8; 32];
        mix.fill(&mut buf).unwrap();
        // 0x0F XOR 0xF0 XOR 0xAA = 0x55. Counter bytes XOR-cancel
        // (3 children's counters XOR to (c) which is non-zero, but
        // the test seeds are deliberately constant per byte position
        // so the only varying input is the counter, and three copies
        // XOR-cancel to one copy of the counter, not zero).
        // Actual: each source emits seed[i&31] ^ (counter & 0xff).
        // Counter is global per source instance; all three sources
        // start at 0 and increment together, so for byte i:
        //   out[i] = (seed_a ^ c) ^ (seed_b ^ c) ^ (seed_c ^ c)
        //          = seed_a ^ seed_b ^ seed_c ^ c
        // (XOR of three identical c's = one c). So output is
        // (0x0F ^ 0xF0 ^ 0xAA) ^ counter_byte_for_position_i.
        let expected: [u8; 32] = core::array::from_fn(|i| {
            (0x0Fu8 ^ 0xF0u8 ^ 0xAAu8) ^ (i as u8)
        });
        assert_eq!(buf, expected);
    }

    /// Child added via `add` participates in the XOR. Same shape as
    /// the two-source test, but the second source joins after
    /// construction.
    #[test]
    fn add_includes_child_in_xor() {
        let mut mix = MixingEntropySource::new(vec![
            Box::new(DeterministicSource::new([0x55u8; 32])),
        ]);
        assert_eq!(mix.len(), 1);
        mix.add(Box::new(DeterministicSource::new([0xAAu8; 32])));
        assert_eq!(mix.len(), 2);
        let mut buf = [0u8; 8];
        mix.fill(&mut buf).unwrap();
        assert_eq!(buf, [0xFFu8; 8]);
    }
}
