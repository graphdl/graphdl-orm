// crates/arest/src/entropy_net.rs
//
// NetworkEntropySource (#583 / Rand-T7). Pull true-random bytes from a
// remote endpoint (random.org atmospheric noise, ANU QRNG quantum
// vacuum, NIST Randomness Beacon) and surface them through the
// `EntropySource` trait.
//
// ## Why a generic fetcher
//
// The arest crate doesn't pull an HTTP client into its dep graph. The
// host CLI (#574) uses `getrandom`, the worker (#572) uses Web Crypto,
// the kernel (#569/#570) uses RDSEED/RNDR — none of them need TLS +
// async + JSON to seed a CSPRNG. Adding `reqwest` here would saddle
// every consumer (kernel, worker, FPGA) with a transitive blob they
// never invoke.
//
// Instead, this module owns the *cache + refill* shape — the policy
// decision that the network is too slow to hit per-call but fast enough
// for periodic batched seed material. Callers supply the actual HTTP
// transport via a fetcher closure: the host CLI hands in a closure
// using `ureq` / `reqwest`; the worker hands in a closure using JS
// `fetch()` shimmed through wasm-bindgen; the kernel hands in a
// closure using its own smoltcp HTTP builder.
//
// Compose with `MixingEntropySource` (#584): in production this should
// never be the only source — network failure must fall back to
// hardware. Install:
//
//   ```text
//   entropy::install(MixingEntropySource::boxed(vec![
//       HostEntropySource::boxed(),                      // OS getrandom
//       NetworkEntropySource::boxed(random_org_fetcher), // atmospheric
//   ]))
//   ```
//
// XOR mix is uniform if either source is uniform — losing the network
// (transient outage, rate-limit, captive portal) silently degrades
// gracefully to OS RNG only.
//
// ## Cache + refill semantics
//
// The fetcher is called with a `request_size_bytes` argument and
// returns a `Vec<u8>` of fresh bytes. The source pulls
// `refill_block_bytes` per fetch (default 4 KiB), serves out of the
// cache for subsequent `fill` calls, and refills when the cache is
// empty. This amortises network latency across many CSPRNG seeds /
// reseeds — first `fill` blocks on one round trip, subsequent fills
// return instantly from cache.
//
// **Failure mode**: a fetcher Err propagates as
// `EntropyError::HardwareUnavailable` — same shape the kernel uses
// when virtio-rng isn't present. Retry policy lives in the fetcher
// itself (a smart fetcher might hold a backoff counter, return cached
// stale bytes from a previous successful pull, etc.); this module is
// stateless across faults.

#[allow(unused_imports)]
use alloc::{boxed::Box, vec::Vec, vec, string::String};

use crate::entropy::{EntropyError, EntropySource};

/// Generic network-fetched entropy source. The `F` type parameter is
/// the fetcher closure — `Fn(usize) -> Result<Vec<u8>, ()>` shape:
/// argument is the number of bytes requested; `Ok(buf)` returns those
/// bytes (caller may return fewer; the source loops); `Err(())` means
/// the fetch failed (network error, rate limit, malformed response).
///
/// `Send + Sync` is required by the trait's contract. The fetcher
/// closure must also be `Send + Sync` so the wrapped source is safely
/// shared across threads behind the global slot's spin lock.
pub struct NetworkEntropySource<F>
where
    F: Fn(usize) -> Result<Vec<u8>, ()> + Send + Sync,
{
    fetcher: F,
    cache: Vec<u8>,
    cursor: usize,
    refill_block_bytes: usize,
}

impl<F> NetworkEntropySource<F>
where
    F: Fn(usize) -> Result<Vec<u8>, ()> + Send + Sync,
{
    /// New source backed by `fetcher`, refilling 4 KiB per network
    /// round-trip. The 4 KiB default amortises one HTTP overhead
    /// (~100 ms typical TLS+RTT) across roughly 128 ChaCha20 reseeds
    /// (32 bytes each), so even a single seed pull saturates the
    /// network cost across the next ~hour of CSPRNG activity at
    /// typical reseed cadence.
    pub fn new(fetcher: F) -> Self {
        Self {
            fetcher,
            cache: Vec::new(),
            cursor: 0,
            refill_block_bytes: 4096,
        }
    }

    /// New source with a custom refill block size. Smaller blocks
    /// reduce first-fill latency (fewer bytes per round-trip) at the
    /// cost of more network calls; larger blocks amortise better but
    /// increase the per-pull payload and quota cost. Set to whatever
    /// the upstream provider's per-request cap allows (random.org's
    /// free tier is 10 000 bytes per call; ANU QRNG is 1024).
    pub fn with_block_size(fetcher: F, refill_block_bytes: usize) -> Self {
        assert!(refill_block_bytes > 0,
            "NetworkEntropySource: refill block size must be > 0");
        Self {
            fetcher,
            cache: Vec::new(),
            cursor: 0,
            refill_block_bytes,
        }
    }

    /// Boxed trait object for `entropy::install`. Accepts the same
    /// fetcher; produces a `Box<dyn EntropySource>` so install sites
    /// don't carry the closure type.
    pub fn boxed(fetcher: F) -> Box<dyn EntropySource>
    where
        F: 'static,
    {
        Box::new(Self::new(fetcher))
    }

    /// Number of bytes available in the cache without a network call.
    /// Diagnostic only — callers shouldn't branch on this except for
    /// logging / metrics.
    pub fn cached_bytes(&self) -> usize {
        self.cache.len().saturating_sub(self.cursor)
    }
}

impl<F> EntropySource for NetworkEntropySource<F>
where
    F: Fn(usize) -> Result<Vec<u8>, ()> + Send + Sync,
{
    /// Fill `buf` from the cache, refilling from the network when the
    /// cache is empty. The first call after construction (or after the
    /// cache drains) blocks on the fetcher; subsequent calls return
    /// from cache until depleted.
    ///
    /// Empty buffer is a no-op and returns `Ok(0)`.
    ///
    /// On fetcher failure: returns `Err(EntropyError::HardwareUnavailable)`
    /// — same shape as kernel's "virtio-rng not present" branch. The
    /// `MixingEntropySource` (#584) treats this as a permanent failure
    /// for that source; if mixed with another working source the mix
    /// also fails (XOR can't proceed without all contributors), so
    /// production install paths should ALSO carry a hardware fallback
    /// installed BEFORE the mixer if network outage tolerance matters
    /// (e.g. mix `[host_getrandom, network]` and the host source
    /// covers the network outage by being installed independently).
    fn fill(&mut self, buf: &mut [u8]) -> Result<usize, EntropyError> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut written = 0;
        while written < buf.len() {
            // Drain cache first. Cursor advances; cache stays
            // alloc'd until the next refill so we don't thrash heap
            // on every fill.
            let available = self.cache.len().saturating_sub(self.cursor);
            if available > 0 {
                let take = (buf.len() - written).min(available);
                let src = &self.cache[self.cursor..self.cursor + take];
                buf[written..written + take].copy_from_slice(src);
                self.cursor += take;
                written += take;
                continue;
            }
            // Cache empty. Refill via the fetcher. Reset cursor +
            // truncate so a partial fetch doesn't leave stale bytes
            // ahead of the new ones.
            self.cache.clear();
            self.cursor = 0;
            match (self.fetcher)(self.refill_block_bytes) {
                Ok(bytes) => {
                    if bytes.is_empty() {
                        // Fetcher claimed success but returned 0 bytes.
                        // Treat as failure — we'd loop forever otherwise.
                        return Err(EntropyError::Fault);
                    }
                    self.cache = bytes;
                }
                Err(()) => return Err(EntropyError::HardwareUnavailable),
            }
        }
        Ok(written)
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};

    /// Synthetic fetcher that returns a deterministic byte pattern
    /// and counts how many times it was invoked. Lets tests assert the
    /// cache + refill semantics without standing up a real HTTP server.
    fn counting_fetcher(
        counter: &'static AtomicUsize,
    ) -> impl Fn(usize) -> Result<Vec<u8>, ()> + Send + Sync {
        move |n| {
            let call = counter.fetch_add(1, Ordering::SeqCst);
            // Distinct bytes per call: byte i of call N = (N * 256 + i) % 256.
            // Lets the assertion distinguish first-call output from refill.
            Ok((0..n).map(|i| ((call * 256 + i) & 0xff) as u8).collect())
        }
    }

    /// Single fill smaller than refill block: fetches once, serves
    /// from cache thereafter.
    #[test]
    fn small_fill_caches_for_subsequent_calls() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        COUNTER.store(0, Ordering::SeqCst);
        let mut source = NetworkEntropySource::with_block_size(
            counting_fetcher(&COUNTER),
            64,
        );
        let mut a = [0u8; 16];
        source.fill(&mut a).unwrap();
        let mut b = [0u8; 16];
        source.fill(&mut b).unwrap();
        // Both pulls came from the same fetcher call (call 0), so
        // `a` is bytes 0..16 of that call and `b` is bytes 16..32.
        assert_eq!(COUNTER.load(Ordering::SeqCst), 1);
        // Cursor advanced; bytes are sequential.
        for (i, byte) in a.iter().enumerate() {
            assert_eq!(*byte, i as u8);
        }
        for (i, byte) in b.iter().enumerate() {
            assert_eq!(*byte, (16 + i) as u8);
        }
    }

    /// Fill larger than the refill block: fetches multiple times
    /// inside one `fill` call and stitches them together.
    #[test]
    fn fill_larger_than_block_refills_until_done() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        COUNTER.store(0, Ordering::SeqCst);
        let mut source = NetworkEntropySource::with_block_size(
            counting_fetcher(&COUNTER),
            8,
        );
        let mut buf = [0u8; 24];
        source.fill(&mut buf).unwrap();
        // 24 bytes / 8 per block = 3 fetcher calls.
        assert_eq!(COUNTER.load(Ordering::SeqCst), 3);
        // First 8 bytes: call 0, bytes 0..8 → 0x00..0x07.
        assert_eq!(&buf[0..8], &[0u8, 1, 2, 3, 4, 5, 6, 7]);
        // Next 8: call 1, bytes 0..8 → (1*256+0)..(1*256+7) & 0xff
        // = 0x00..0x07 (the call index doesn't bleed into the byte
        // value because 256 mod 256 = 0). Same pattern repeats.
        assert_eq!(&buf[8..16], &[0u8, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(&buf[16..24], &[0u8, 1, 2, 3, 4, 5, 6, 7]);
    }

    /// Fetcher Err propagates as HardwareUnavailable. Subsequent
    /// fills can succeed (the source is stateless across faults; the
    /// fetcher itself owns retry policy).
    #[test]
    fn fetcher_failure_surfaces_as_hardware_unavailable() {
        let mut source = NetworkEntropySource::new(|_| Err(()));
        let mut buf = [0u8; 8];
        assert_eq!(source.fill(&mut buf), Err(EntropyError::HardwareUnavailable));
    }

    /// Fetcher returning empty Vec (success but zero bytes) surfaces
    /// as Fault — we'd loop forever otherwise.
    #[test]
    fn empty_fetcher_response_surfaces_as_fault() {
        let mut source = NetworkEntropySource::new(|_| Ok(Vec::new()));
        let mut buf = [0u8; 8];
        assert_eq!(source.fill(&mut buf), Err(EntropyError::Fault));
    }

    /// Empty buffer: no-op return, no fetcher call.
    #[test]
    fn empty_buffer_is_noop() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        COUNTER.store(0, Ordering::SeqCst);
        let mut source = NetworkEntropySource::new(counting_fetcher(&COUNTER));
        let mut buf: [u8; 0] = [];
        assert_eq!(source.fill(&mut buf), Ok(0));
        assert_eq!(COUNTER.load(Ordering::SeqCst), 0);
    }

    /// Compose with MixingEntropySource (#584). Network source +
    /// deterministic source XORed produces XOR of their outputs.
    /// Verifies the trait-object boxing path used at install time.
    #[test]
    fn composes_with_mixing_entropy_source() {
        use crate::entropy::DeterministicSource;
        use crate::entropy_mix::MixingEntropySource;

        let mut mix = MixingEntropySource::new(vec![
            Box::new(NetworkEntropySource::new(|n| {
                Ok(vec![0xAAu8; n])  // constant 0xAA per byte
            })),
            Box::new(DeterministicSource::new([0xAAu8; 32])),
        ]);
        let mut buf = [0u8; 32];
        mix.fill(&mut buf).unwrap();
        // (0xAA constant) XOR (DeterministicSource output for byte i =
        // 0xAA ^ counter_byte) = counter_byte = 0..32.
        // Wait — DeterministicSource output byte i is `seed[i&31] ^
        // (counter & 0xff)`, where counter starts at 0 and increments
        // per byte. So byte 0 = 0xAA ^ 0 = 0xAA; byte 1 = 0xAA ^ 1
        // = 0xAB; etc. XOR with constant 0xAA gives 0..32.
        let expected: [u8; 32] = core::array::from_fn(|i| i as u8);
        assert_eq!(buf, expected);
    }
}
