// crates/arest/src/crypto.rs
//
// Event signing primitives (AREST §5.5 — Distributed Evaluation).
//
// Per the AREST paper: "For anonymous peers, events carry cryptographic
// signatures for identity." This module supplies HMAC-SHA256 signing so
// Commands can carry a `signature` alongside `sender` and the engine
// can verify sender/payload integrity.
//
// Key source: AREST_HMAC_KEY env var at runtime. Falls back to a
// compile-time dev key when the env var is absent (dev/test only).
//
// ## Random-byte path (#578)
//
// The audit-log / snap-id signing path (#331 Sec-4) needs nonces and
// per-tenant secrets that must NOT come from a separate RNG: the engine
// goal is one entropy slot, one seeded CSPRNG, one source of bytes
// everywhere. `crypto::random_bytes` and `crypto::random_secret` are
// the public entry points crypto callers reach for; both delegate to
// `crate::csprng::random_bytes`, which in turn pulls a 32-byte seed
// from the installed `entropy::EntropySource`. Targets install their
// source at boot (e.g. `cli::entropy_host::HostEntropySource` for the
// host CLI in `main.rs`); the panic-on-uninstalled-source contract is
// inherited from csprng (silent zeroes are the worst possible failure
// mode for a nonce).

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

type HmacSha256 = Hmac<Sha256>;

/// Dev-only fallback key. Production MUST set AREST_HMAC_KEY.
const DEV_KEY: &[u8] = b"AREST-DEV-KEY-NOT-FOR-PRODUCTION";

/// Separator between sender and payload in the pre-image.
const SEP: &str = "::";

/// Get the signing key from env or fall back to dev key. Under
/// `no_std` there is no environment — the kernel-side caller sets
/// the key via a different channel (baked constant, tenant record),
/// so we just return the dev key there and trust the caller to
/// override at a higher layer.
#[cfg(not(feature = "no_std"))]
fn key() -> Vec<u8> {
    std::env::var("AREST_HMAC_KEY")
        .map(|k| k.into_bytes())
        .unwrap_or_else(|_| DEV_KEY.to_vec())
}

#[cfg(feature = "no_std")]
fn key() -> Vec<u8> {
    DEV_KEY.to_vec()
}

/// Compute HMAC-SHA256 over (sender || SEP || payload).
/// Returns lowercase hex-encoded 64-character digest.
pub fn sign(sender: &str, payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(&key())
        .expect("HMAC accepts any key length");
    mac.update(sender.as_bytes());
    mac.update(SEP.as_bytes());
    mac.update(payload.as_bytes());
    let result = mac.finalize().into_bytes();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Verify that `signature` matches HMAC-SHA256 of (sender, payload).
/// Uses constant-time comparison to prevent timing oracles.
pub fn verify_signature(sender: &str, payload: &str, signature: &str) -> bool {
    let expected = sign(sender, payload);
    // Both are hex strings — constant-time compare on bytes.
    expected.len() == signature.len()
        && expected.as_bytes().ct_eq(signature.as_bytes()).into()
}

/// HMAC-SHA256 with a caller-supplied key. Returns the full 64-char
/// lowercase-hex digest; callers that only need a shorter tag truncate
/// the returned string (e.g. `[..16]` for a 64-bit tag, as Sec-4
/// snapshot-id signing does).
pub fn sign_with(key: &[u8], data: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key)
        .expect("HMAC accepts any key length");
    mac.update(data);
    let result = mac.finalize().into_bytes();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Constant-time equality on two string slices. Length mismatch
/// short-circuits to `false` (length is public; the early return is
/// not timing-sensitive); equal-length compares go through
/// `subtle::ct_eq` so partial-prefix matches don't leak timing.
pub fn ct_eq_str(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

/// Fill `buf` with cryptographically secure random bytes drawn from the
/// process-wide CSPRNG (`crate::csprng`). All crypto-side random byte
/// sites — nonces for HMAC pre-images, per-tenant `snapshot_secret`
/// material (#331 Sec-4), audit-log nonces — MUST funnel through this
/// helper rather than reaching for `rand::*` / `getrandom::*` /
/// hardcoded counters. The single funnel is the #578 contract: one
/// entropy slot, one seeded CSPRNG, one source of bytes.
///
/// Panics if no entropy source has been installed (inherited from
/// `csprng::random_bytes`). Targets install one at boot — the host CLI
/// wires `cli::entropy_host::HostEntropySource` from `main.rs` — so a
/// panic here points to a target that skipped the install step rather
/// than a runtime entropy outage.
pub fn random_bytes(buf: &mut [u8]) {
    crate::csprng::random_bytes(buf);
}

/// Generate a fresh 32-byte secret from the CSPRNG. Convenience over
/// `random_bytes(&mut [0u8; 32])` for the common per-tenant /
/// per-snapshot HMAC key shape — matches the 256-bit input width of
/// HMAC-SHA256, large enough that an attacker who can only observe
/// signed tags cannot enumerate the keyspace.
///
/// Same panic contract as `random_bytes` (no entropy source → fail
/// loud during target bring-up rather than silently emitting zeroes).
pub fn random_secret() -> [u8; 32] {
    let mut secret = [0u8; 32];
    random_bytes(&mut secret);
    secret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_signature_verifies() {
        let sender = "alice@example.com";
        let payload = r#"{"type":"CreateEntity","noun":"Order","id":"ord-1"}"#;
        let sig = sign(sender, payload);
        assert_eq!(sig.len(), 64, "HMAC-SHA256 hex digest is 64 chars");
        assert!(verify_signature(sender, payload, &sig),
            "valid signature must verify");
    }

    #[test]
    fn invalid_signature_rejected() {
        let sender = "alice@example.com";
        let payload = r#"{"type":"CreateEntity","noun":"Order","id":"ord-1"}"#;
        assert!(!verify_signature(sender, payload, &"0".repeat(64)),
            "bogus signature must fail verification");
        assert!(!verify_signature(sender, payload, ""),
            "empty signature must fail verification");
    }

    #[test]
    fn tampered_payload_fails() {
        let sender = "alice@example.com";
        let payload_original = r#"{"noun":"Order","id":"ord-1"}"#;
        let payload_tampered = r#"{"noun":"Order","id":"ord-2"}"#;
        let sig = sign(sender, payload_original);
        assert!(!verify_signature(sender, payload_tampered, &sig),
            "signature over ord-1 must not verify against ord-2");
    }

    #[test]
    fn tampered_sender_fails() {
        let payload = r#"{"noun":"Order","id":"ord-1"}"#;
        let sig = sign("alice@example.com", payload);
        assert!(!verify_signature("mallory@evil.com", payload, &sig),
            "alice's signature must not verify for mallory");
    }

    #[test]
    fn sign_is_deterministic() {
        let a = sign("u", "p");
        let b = sign("u", "p");
        assert_eq!(a, b, "sign must be a pure function");
    }

    #[test]
    fn sign_distinguishes_fields() {
        let s1 = sign("ab", "c");
        let s2 = sign("a", "bc");
        assert_ne!(s1, s2,
            "SEP between sender and payload must prevent field confusion");
    }

    // ── Random-byte path (#578) ─────────────────────────────────────
    //
    // These tests pin the contract that `crypto::random_bytes` /
    // `crypto::random_secret` route through `csprng` (and therefore
    // through the single installed `entropy::EntropySource`). The
    // fixture installs a `DeterministicSource`, forces a CSPRNG
    // reseed, runs the body, and clears the source — same pattern
    // as `csprng::tests::with_deterministic_csprng`. The cross-module
    // `entropy::TEST_LOCK` keeps a concurrent
    // `entropy::tests::*` case from sabotaging the fixture mid-fill.
    use crate::entropy::{self, DeterministicSource};

    fn with_deterministic_entropy<F: FnOnce()>(seed: [u8; 32], body: F) {
        let _guard = entropy::TEST_LOCK.lock();
        entropy::install(Box::new(DeterministicSource::new(seed)));
        crate::csprng::reseed();
        body();
        entropy::uninstall();
        crate::csprng::reseed();
    }

    #[test]
    fn random_bytes_routes_through_csprng() {
        // Same seed via crypto::random_bytes vs csprng::random_bytes
        // must produce identical streams — proves the helper is a
        // pure delegate, no side state of its own.
        let mut via_crypto = [0u8; 32];
        let mut via_csprng = [0u8; 32];
        with_deterministic_entropy([21u8; 32], || {
            random_bytes(&mut via_crypto);
        });
        with_deterministic_entropy([21u8; 32], || {
            crate::csprng::random_bytes(&mut via_csprng);
        });
        assert_eq!(via_crypto, via_csprng,
            "crypto::random_bytes must be a pure delegate to csprng::random_bytes");
    }

    #[test]
    fn random_bytes_is_not_all_zero() {
        // Non-zero seed through ChaCha20 must produce non-zero output;
        // an all-zero buffer here would be the smoking gun for either
        // a broken csprng wire-up or a silently-uninstalled source.
        with_deterministic_entropy([23u8; 32], || {
            let mut buf = [0u8; 64];
            random_bytes(&mut buf);
            assert!(buf.iter().any(|&b| b != 0),
                "csprng output for non-zero seed must not be all zero");
        });
    }

    #[test]
    fn random_bytes_changes_with_seed() {
        // Different entropy seeds must produce different output —
        // protects against a future regression where the helper
        // accidentally reads from a fixed buffer.
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        with_deterministic_entropy([25u8; 32], || { random_bytes(&mut a); });
        with_deterministic_entropy([26u8; 32], || { random_bytes(&mut b); });
        assert_ne!(a, b,
            "different entropy seeds must yield different csprng output");
    }

    #[test]
    fn random_secret_is_thirty_two_bytes() {
        // The HMAC-SHA256 key width is 256 bits; the helper must
        // hand back exactly that. Compile-time check via the array
        // type, but pin behaviour with an explicit assert.
        with_deterministic_entropy([27u8; 32], || {
            let s = random_secret();
            assert_eq!(s.len(), 32);
            assert!(s.iter().any(|&b| b != 0),
                "random_secret must not return all-zero for a non-zero seed");
        });
    }

    #[test]
    fn random_secret_advances_per_call() {
        // Two successive calls under one entropy session must yield
        // distinct secrets — the CSPRNG counter advances per block,
        // so two 32-byte draws come from disjoint stream offsets.
        with_deterministic_entropy([29u8; 32], || {
            let a = random_secret();
            let b = random_secret();
            assert_ne!(a, b,
                "successive random_secret calls must advance the csprng stream");
        });
    }

    #[test]
    fn random_secret_seeds_hmac_path() {
        // End-to-end: a secret drawn from the csprng feeds sign_with
        // and verify-via-ct_eq_str round-trips. Catches regressions
        // where random_secret would hand back a key that breaks the
        // HMAC primitive (zeroed key, wrong length, etc.).
        with_deterministic_entropy([31u8; 32], || {
            let key = random_secret();
            let payload = b"snap-1";
            let tag = sign_with(&key, payload);
            assert_eq!(tag.len(), 64, "HMAC-SHA256 hex digest is 64 chars");
            // Re-sign with the same key + payload must reproduce
            // the same tag (HMAC is deterministic given fixed key).
            let again = sign_with(&key, payload);
            assert!(ct_eq_str(&tag, &again),
                "HMAC over csprng-derived key must be deterministic given fixed key + payload");
        });
    }
}
