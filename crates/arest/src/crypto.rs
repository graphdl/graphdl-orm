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
}
