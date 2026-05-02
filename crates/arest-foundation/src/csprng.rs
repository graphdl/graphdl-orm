// crates/arest/src/csprng.rs
//
// Hand-rolled ChaCha20 cryptographically secure pseudo-random number
// generator. Pair to `entropy.rs` (#567/#568): entropy.rs supplies the
// seed, this module turns it into the uniform stream every consumer
// reads from.
//
// ## Why hand-rolled
//
// The kernel's dep tree is intentionally tight (#565 audit). Pulling
// the `chacha20` crate would add a dep that the FPGA generator must
// also account for and that ripples into every downstream consumer's
// MSRV / no_std story. ChaCha20 is ~80 lines of bit-twiddling in Rust;
// the algorithm is in RFC 8439 and has no patent claims. Hand-roll.
//
// ## Algorithm summary (RFC 8439, Section 2)
//
// State is a 4×4 matrix of u32 (16 words = 64 bytes per block):
//
//     "expa" "nd 3" "2-by" "te k"   <- 4 constants
//     k0     k1     k2     k3       <- 8 words of key
//     k4     k5     k6     k7
//     ctr    n0     n1     n2       <- 1 counter + 3 nonce words
//
// Twenty rounds (= 10 doublerounds), each doubleround is:
//   - 4 column quarter-rounds
//   - 4 diagonal quarter-rounds
//
// Add the original input state back to the working state after the
// rounds (Davies-Meyer step). Output is the 64-byte block. Counter
// increments per block.
//
// ## Reseed policy (forward security)
//
// After OUTPUT_BUDGET bytes (1 MiB) have been produced, the next
// `random_*` call pulls 32 fresh bytes from `entropy::GLOBAL_SOURCE`
// and re-keys. Forward-security goal: an adversary who compromises
// state after a reseed cannot reconstruct output from before. The
// 1 MiB number is conservative — RFC recommends max 2^32 blocks per
// nonce (256 GiB), but reseeding sooner narrows the rewind window
// further.
//
// ## Concurrency
//
// One process-wide state behind a `spin::Mutex`. Each `random_*` call
// is short (one atomic counter check + a 64-byte block emit at most),
// so spinning is cheap. The mutex pattern matches `entropy::GLOBAL_SOURCE`
// — same ergonomics on no_std as on std.

#[allow(unused_imports)]
use alloc::{boxed::Box, string::{String, ToString}, vec::Vec, format};
use crate::entropy;

// ── Tunables ────────────────────────────────────────────────────────

/// After this many output bytes, the next call reseeds from the
/// installed entropy source. 1 MiB picked to limit the forward-security
/// rewind window without making reseed traffic dominate workloads that
/// only need a few KB of randomness per process lifetime.
const OUTPUT_BUDGET: u64 = 1 << 20;

/// Bytes per ChaCha20 block (4×4 u32 words).
const BLOCK_SIZE: usize = 64;

// ── State ───────────────────────────────────────────────────────────

/// ChaCha20 working state. The first 4 words are the fixed "expand
/// 32-byte k" constant; key occupies words 4-11; counter is word 12;
/// the nonce occupies words 13-15.
///
/// `seeded == false` means "no key material has been mixed in yet" —
/// the lazy-seed path runs on the first `random_*` call.
struct CsprngState {
    state: [u32; 16],
    seeded: bool,
    /// Bytes already emitted under the current key. When this exceeds
    /// `OUTPUT_BUDGET`, the next call reseeds.
    emitted: u64,
    /// Buffered, partially-consumed block. `buf_pos` indexes into
    /// `buf` for the next byte to hand out; when `buf_pos == 64`, the
    /// next call generates a fresh block.
    buf: [u8; BLOCK_SIZE],
    buf_pos: usize,
}

impl CsprngState {
    /// Empty state — seeded lazily on first use. The `expand 32-byte k`
    /// constant is fixed by RFC 8439; the rest is zeroed and overwritten
    /// when a real key arrives.
    const fn new() -> Self {
        Self {
            state: [
                0x61707865, 0x3320646e, 0x79622d32, 0x6b206574,
                0, 0, 0, 0, 0, 0, 0, 0,
                0,
                0, 0, 0,
            ],
            seeded: false,
            emitted: 0,
            buf: [0u8; BLOCK_SIZE],
            buf_pos: BLOCK_SIZE,
        }
    }
}

static STATE: spin::Mutex<CsprngState> = spin::Mutex::new(CsprngState::new());

// ── Public API ──────────────────────────────────────────────────────

/// Force a reseed on the next `random_*` call. Useful for explicit
/// post-fork / post-snapshot-restore hygiene where the caller knows
/// the in-memory key may have been duplicated. Production callers
/// rarely need this — the 1 MiB output budget normally suffices.
pub fn reseed() {
    let mut s = STATE.lock();
    s.seeded = false;
    s.emitted = 0;
    s.buf_pos = BLOCK_SIZE;
}

/// Fill `buf` with cryptographically secure random bytes.
///
/// Panics if no entropy source is installed and a seed is needed. This
/// is intentional: the alternative — silently handing back zeroed or
/// stale bytes — produces hard-to-debug downstream symptoms (libc
/// stack canary always 0, identical "unique" IDs across processes).
/// Targets MUST install a source before any callable path can reach
/// the CSPRNG; the panic gives those targets a fast feedback loop
/// during bring-up.
pub fn random_bytes(buf: &mut [u8]) {
    let mut state = STATE.lock();
    fill_locked(&mut state, buf);
}

/// Convenience helper for callers that want one u64. Identical
/// semantics to `random_bytes(&mut [u8; 8])` then little-endian
/// decode — provided as its own entry so call sites that scatter
/// many u64 reads don't pay an 8-byte stack churn each.
pub fn random_u64() -> u64 {
    let mut buf = [0u8; 8];
    random_bytes(&mut buf);
    u64::from_le_bytes(buf)
}

// ── Internal: locked fill / seed / block ────────────────────────────

/// Fill `buf` while holding the state lock. Lazy-seed runs once if
/// `seeded == false`; reseed runs when `emitted >= OUTPUT_BUDGET`.
/// After seeding/reseeding the function copies as much of the buffered
/// block as fits, regenerates blocks for any remaining bytes, and
/// updates `emitted` accordingly.
fn fill_locked(state: &mut CsprngState, buf: &mut [u8]) {
    if !state.seeded {
        seed_from_entropy(state);
    } else if state.emitted >= OUTPUT_BUDGET {
        // Force-reseed; preserves the seeded bit so the panic-on-
        // missing-source path doesn't fire under the steady-state
        // budget rotation (where the entropy source IS present).
        seed_from_entropy(state);
    }

    let mut written = 0;
    while written < buf.len() {
        // Refill the block buffer if exhausted.
        if state.buf_pos >= BLOCK_SIZE {
            chacha20_block(&state.state, &mut state.buf);
            // Increment the 32-bit counter (word 12). Wraparound is
            // fine for our budget — we reseed long before 2^32 blocks.
            state.state[12] = state.state[12].wrapping_add(1);
            state.buf_pos = 0;
        }

        let take = (buf.len() - written).min(BLOCK_SIZE - state.buf_pos);
        buf[written..written + take]
            .copy_from_slice(&state.buf[state.buf_pos..state.buf_pos + take]);
        state.buf_pos += take;
        written += take;
    }

    state.emitted = state.emitted.saturating_add(buf.len() as u64);
}

/// Pull 32 bytes from the installed entropy source and re-key the
/// ChaCha20 state. Counter resets to 0; nonce stays zero (a fresh key
/// per reseed makes nonce reuse harmless within a single process).
/// Buffer is invalidated so the next emit produces a fresh block.
///
/// Panics if no entropy source is installed — see `random_bytes` for
/// the rationale.
fn seed_from_entropy(state: &mut CsprngState) {
    if !entropy::is_installed() {
        panic!(
            "arest::csprng: no entropy source installed. \
             Targets must call entropy::install(...) during boot \
             before any random_bytes / random_u64 call."
        );
    }
    let mut key = [0u8; 32];
    entropy::fill(&mut key)
        .expect("entropy source installed but failed during reseed");

    // Re-key: words 4..12 are the 8 key words, little-endian.
    for (i, chunk) in key.chunks_exact(4).enumerate() {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(chunk);
        state.state[4 + i] = u32::from_le_bytes(bytes);
    }
    // Reset counter (word 12) and nonce (words 13..16). Fresh key →
    // safe to reuse nonce 0 for this generation.
    state.state[12] = 0;
    state.state[13] = 0;
    state.state[14] = 0;
    state.state[15] = 0;
    state.seeded = true;
    state.emitted = 0;
    state.buf_pos = BLOCK_SIZE;
}

// ── ChaCha20 block function (RFC 8439 §2.3) ─────────────────────────

/// Generate one 64-byte ChaCha20 block from `input` into `out`. Pure
/// function of the input state — no I/O, no shared state, no panics.
/// Used by `fill_locked` per output block.
fn chacha20_block(input: &[u32; 16], out: &mut [u8; BLOCK_SIZE]) {
    let mut working = *input;
    // 10 doublerounds = 20 rounds.
    for _ in 0..10 {
        // Column rounds.
        quarter_round(&mut working, 0, 4, 8, 12);
        quarter_round(&mut working, 1, 5, 9, 13);
        quarter_round(&mut working, 2, 6, 10, 14);
        quarter_round(&mut working, 3, 7, 11, 15);
        // Diagonal rounds.
        quarter_round(&mut working, 0, 5, 10, 15);
        quarter_round(&mut working, 1, 6, 11, 12);
        quarter_round(&mut working, 2, 7, 8, 13);
        quarter_round(&mut working, 3, 4, 9, 14);
    }
    // Davies-Meyer: add the original input state to the working state.
    // Without this, the block function would be invertible (and thus
    // unsuitable as a stream cipher) — adding the input mixes one-way.
    for i in 0..16 {
        working[i] = working[i].wrapping_add(input[i]);
    }
    // Serialise as little-endian.
    for (i, word) in working.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
}

/// One ChaCha20 quarter-round on four state words. Body is exactly
/// the four ARX (add-rotate-xor) lines from RFC 8439 §2.1 — kept in
/// one inline-friendly function so the block loop above reads as
/// "8 quarter-rounds × 10 doublerounds".
#[inline(always)]
fn quarter_round(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]); s[d] ^= s[a]; s[d] = s[d].rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]); s[b] ^= s[c]; s[b] = s[b].rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]); s[d] ^= s[a]; s[d] = s[d].rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]); s[b] ^= s[c]; s[b] = s[b].rotate_left(7);
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entropy::{self, DeterministicSource};

    /// Set up a deterministic entropy source and clear CSPRNG state so
    /// tests get reproducible output regardless of run order. Cleanup
    /// (uninstall + reseed reset) at the end keeps the next test from
    /// inheriting our fixture.
    ///
    /// Holds the cross-module `entropy::TEST_LOCK` for the entire body.
    /// `cargo test` runs cases in parallel; without the lock, a
    /// concurrent `entropy::install(AlwaysFault)` from one of the
    /// `entropy::tests::*` cases would sabotage a csprng fill mid-stream.
    fn with_deterministic_csprng<F: FnOnce()>(seed: [u8; 32], body: F) {
        let _guard = entropy::TEST_LOCK.lock();
        entropy::install(Box::new(DeterministicSource::new(seed)));
        // Force a reseed on the next call so we definitely get bytes
        // derived from `seed` rather than whatever an earlier test
        // left in STATE.
        reseed();
        body();
        // Restore the lock to a "no source / unseeded" state for any
        // subsequent test that probes the panic path.
        entropy::uninstall();
        reseed();
    }

    /// RFC 8439 §2.1.1 quarter-round test vector. Verifies the ARX
    /// inner loop matches the spec — without this, every higher-level
    /// test would be downstream of a broken primitive.
    #[test]
    fn quarter_round_rfc_vector() {
        // RFC 8439 §2.1.1: input (a, b, c, d) =
        // (0x11111111, 0x01020304, 0x9b8d6f43, 0x01234567)
        // expected output =
        // (0xea2a92f4, 0xcb1cf8ce, 0x4581472e, 0x5881c4bb)
        let mut s = [0u32; 16];
        s[0] = 0x11111111;
        s[1] = 0x01020304;
        s[2] = 0x9b8d6f43;
        s[3] = 0x01234567;
        quarter_round(&mut s, 0, 1, 2, 3);
        assert_eq!(s[0], 0xea2a92f4);
        assert_eq!(s[1], 0xcb1cf8ce);
        assert_eq!(s[2], 0x4581472e);
        assert_eq!(s[3], 0x5881c4bb);
    }

    /// RFC 8439 §2.3.2 full block test vector. Confirms the block
    /// function (column + diagonal rounds + Davies-Meyer) matches the
    /// spec end to end. This is the test that catches "off-by-one in
    /// rotation count" / "swapped column-vs-diagonal" bugs.
    #[test]
    fn chacha20_block_rfc_vector() {
        // RFC 8439 §2.3.2 input.
        let key: [u8; 32] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
            0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
        ];
        let nonce: [u8; 12] = [
            0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x4a,
            0x00, 0x00, 0x00, 0x00,
        ];
        let counter: u32 = 1;

        let mut state = [
            0x61707865, 0x3320646e, 0x79622d32, 0x6b206574,
            0, 0, 0, 0, 0, 0, 0, 0,
            counter,
            0, 0, 0,
        ];
        for (i, chunk) in key.chunks_exact(4).enumerate() {
            state[4 + i] = u32::from_le_bytes(chunk.try_into().unwrap());
        }
        for (i, chunk) in nonce.chunks_exact(4).enumerate() {
            state[13 + i] = u32::from_le_bytes(chunk.try_into().unwrap());
        }

        let mut out = [0u8; BLOCK_SIZE];
        chacha20_block(&state, &mut out);

        // Expected first 16 bytes per RFC 8439 §2.3.2:
        // 10 f1 e7 e4 d1 3b 59 15 50 0f dd 1f a3 20 71 c4
        let expected_prefix: [u8; 16] = [
            0x10, 0xf1, 0xe7, 0xe4, 0xd1, 0x3b, 0x59, 0x15,
            0x50, 0x0f, 0xdd, 0x1f, 0xa3, 0x20, 0x71, 0xc4,
        ];
        assert_eq!(&out[..16], &expected_prefix,
            "chacha20 block output must match RFC 8439 §2.3.2 vector");
    }

    #[test]
    fn random_bytes_with_deterministic_source_is_deterministic() {
        let mut buf_a = [0u8; 16];
        let mut buf_b = [0u8; 16];
        with_deterministic_csprng([1u8; 32], || {
            random_bytes(&mut buf_a);
        });
        with_deterministic_csprng([1u8; 32], || {
            random_bytes(&mut buf_b);
        });
        assert_eq!(buf_a, buf_b,
            "same entropy seed must produce same CSPRNG output");
    }

    #[test]
    fn random_bytes_changes_seed_changes_output() {
        let mut buf_a = [0u8; 16];
        let mut buf_b = [0u8; 16];
        with_deterministic_csprng([1u8; 32], || { random_bytes(&mut buf_a); });
        with_deterministic_csprng([2u8; 32], || { random_bytes(&mut buf_b); });
        assert_ne!(buf_a, buf_b,
            "different seeds must produce different output");
    }

    #[test]
    fn random_u64_advances() {
        with_deterministic_csprng([3u8; 32], || {
            let a = random_u64();
            let b = random_u64();
            // Counter advances per call → values must differ.
            assert_ne!(a, b);
        });
    }

    #[test]
    fn random_bytes_is_not_all_zero() {
        with_deterministic_csprng([5u8; 32], || {
            let mut buf = [0u8; 64];
            random_bytes(&mut buf);
            assert!(buf.iter().any(|&b| b != 0),
                "ChaCha20 output for non-zero seed must not be all zero");
        });
    }

    #[test]
    fn random_bytes_handles_buffer_smaller_than_block() {
        with_deterministic_csprng([7u8; 32], || {
            // 17 bytes < 64-byte block — exercises the "partial buffer
            // consumed, rest stays for next call" path.
            let mut a = [0u8; 17];
            let mut b = [0u8; 17];
            random_bytes(&mut a);
            random_bytes(&mut b);
            assert_ne!(a, b, "successive sub-block reads must advance");
        });
    }

    #[test]
    fn random_bytes_handles_buffer_spanning_blocks() {
        with_deterministic_csprng([9u8; 32], || {
            // 200 bytes > 64-byte block — exercises the multi-block
            // emit loop. The check is that no panic + no all-zero.
            let mut buf = [0u8; 200];
            random_bytes(&mut buf);
            assert!(buf.iter().any(|&b| b != 0));
            // Spread check: the first and last 64-byte windows must
            // differ (they come from different blocks).
            assert_ne!(&buf[..64], &buf[136..200]);
        });
    }

    #[test]
    fn reseed_after_budget_changes_state() {
        with_deterministic_csprng([11u8; 32], || {
            // Drain past OUTPUT_BUDGET to force a reseed. CHUNK = 4 KiB
            // so the loop count is small; iterations = budget/chunk+1
            // guarantees the next call's "emitted >= OUTPUT_BUDGET"
            // check fires.
            const CHUNK: usize = 4096;
            let mut scratch = [0u8; CHUNK];
            let iterations = (OUTPUT_BUDGET as usize / CHUNK) + 1;
            for _ in 0..iterations {
                random_bytes(&mut scratch);
            }

            // The probe: the test must NOT lock up (panic on the
            // following call would be the smoking gun for a broken
            // reseed loop) and the state must be bounded — `emitted`
            // is reset to 0 each reseed, so the running count after
            // any number of reseeds + a final fill is at most one
            // CHUNK + the trailing read. Crucially, `emitted` must be
            // STRICTLY LESS than the total bytes ever produced
            // (`iterations * CHUNK + 16`), proving at least one
            // reseed event has happened.
            let mut more = [0u8; 16];
            random_bytes(&mut more);
            assert!(more.iter().any(|&b| b != 0),
                "post-reseed output must be non-zero for non-zero seed");

            let s = STATE.lock();
            let total_produced = (iterations * CHUNK + more.len()) as u64;
            assert!(s.emitted < total_produced,
                "emitted ({}) must be < total bytes produced ({}) — \
                 at least one reseed should have reset the counter",
                s.emitted, total_produced);
            // Bounded: at most one chunk plus the trailing 16-byte
            // read since the last reseed (the reseed happens at the
            // top of fill_locked, before the new bytes are counted,
            // so a single chunk after reseed is the max).
            assert!(s.emitted <= (CHUNK + more.len()) as u64,
                "emitted ({}) should reflect only post-most-recent-reseed bytes \
                 (≤ {} = CHUNK + more.len())",
                s.emitted, CHUNK + more.len());
        });
    }

    #[test]
    fn explicit_reseed_clears_seeded_bit() {
        with_deterministic_csprng([13u8; 32], || {
            // Generate some bytes to ensure seeded == true, emitted > 0.
            let mut warm = [0u8; 8];
            random_bytes(&mut warm);
            assert!(STATE.lock().seeded);
            assert!(STATE.lock().emitted > 0);

            reseed();
            assert!(!STATE.lock().seeded, "reseed clears seeded bit");

            // Next call should re-seed without panic (source still
            // installed via with_deterministic_csprng).
            let mut after = [0u8; 8];
            random_bytes(&mut after);
            assert!(STATE.lock().seeded, "next call after reseed re-seeds");
        });
    }

    /// When no entropy source is installed AND the CSPRNG has not yet
    /// been seeded, the next `random_*` call must panic — the
    /// alternative is silent zero output, which is the worst possible
    /// failure mode for downstream consumers (stack canaries,
    /// session IDs).
    #[test]
    #[should_panic(expected = "no entropy source installed")]
    fn random_bytes_panics_when_no_source_and_unseeded() {
        // Lock-guarded so a concurrent test isn't installing a source
        // mid-flight while we're trying to prove the no-source panic.
        // NOTE: this test panics with the lock held; spin::Mutex has
        // no poison state, so subsequent tests still acquire cleanly
        // — this is one reason the crate uses spin over std locks.
        let _guard = entropy::TEST_LOCK.lock();
        entropy::uninstall();
        reseed();
        let mut buf = [0u8; 1];
        random_bytes(&mut buf);
    }

    /// Multiple short reads must not corrupt the buffered block — the
    /// buf_pos cursor handles cross-call partial consumption. Reads
    /// the same block twice via two short calls vs. one big call and
    /// compares.
    #[test]
    fn buffered_block_partial_consume_is_consistent() {
        let mut combined = [0u8; 32];
        with_deterministic_csprng([15u8; 32], || {
            // One 32-byte read.
            random_bytes(&mut combined);
        });
        let mut split = [0u8; 32];
        with_deterministic_csprng([15u8; 32], || {
            // Two 16-byte reads → must yield the same 32 bytes.
            random_bytes(&mut split[..16]);
            random_bytes(&mut split[16..]);
        });
        assert_eq!(combined, split,
            "split reads must produce the same stream as a single read");
    }
}
