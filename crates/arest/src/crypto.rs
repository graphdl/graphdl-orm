// crates/arest/src/crypto.rs
//
// Event signing primitives (AREST §5.5 — Distributed Evaluation).
//
// Per the AREST paper: "For anonymous peers, events carry cryptographic
// signatures for identity." This module supplies a minimal MAC primitive
// so Commands can carry a `signature` alongside `sender` and the engine
// can verify sender/payload integrity without pulling in a heavy crypto
// dependency.
//
// IMPORTANT: this is a PLACEHOLDER using std::hash::DefaultHasher. It is
// NOT cryptographically secure. The architecture (sign / verify over
// sender + payload + secret) is correct and swap-in-ready: replacing the
// inner hash with HMAC-SHA256 is a one-function change. Until then, do
// NOT rely on these signatures for adversarial peer authentication — use
// them to demonstrate the wiring and enforce the flow.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Compile-time shared secret. In production this MUST be replaced with
/// a per-peer or per-domain keying material negotiated out-of-band.
const SECRET: &str = "AREST-PLACEHOLDER-SECRET-v0";

/// Separator used when concatenating fields into the pre-image. Chosen
/// so that field boundaries cannot be trivially confused (no plain
/// `sender|payload` ambiguity when either contains `|`).
const SEP: &str = "::";

/// Derive the canonical pre-image for a (sender, payload) pair.
/// Pure function, no allocation on the hasher path beyond the owned String.
fn preimage(sender: &str, payload: &str) -> String {
    let mut s = String::with_capacity(sender.len() + payload.len() + SECRET.len() + 2 * SEP.len());
    s.push_str(sender);
    s.push_str(SEP);
    s.push_str(payload);
    s.push_str(SEP);
    s.push_str(SECRET);
    s
}

/// Compute a deterministic MAC over (sender, payload, SECRET).
/// Returns a lowercase hex-encoded u64 — stable across runs of the same
/// binary on the same platform. See module note: placeholder only.
pub fn sign(sender: &str, payload: &str) -> String {
    let pre = preimage(sender, payload);
    let mut h = DefaultHasher::new();
    pre.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Verify that `signature` matches the MAC of (sender, payload).
/// Returns true iff the signature is well-formed and matches.
/// Constant-time comparison is NOT required for the placeholder, but
/// the real HMAC implementation MUST use `subtle::ConstantTimeEq` or
/// equivalent to prevent timing oracles.
pub fn verify_signature(sender: &str, payload: &str, signature: &str) -> bool {
    let expected = sign(sender, payload);
    expected.as_bytes() == signature.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_signature_verifies() {
        let sender = "alice@example.com";
        let payload = r#"{"type":"CreateEntity","noun":"Order","id":"ord-1"}"#;
        let sig = sign(sender, payload);
        assert!(verify_signature(sender, payload, &sig),
            "valid signature must verify");
    }

    #[test]
    fn invalid_signature_rejected() {
        let sender = "alice@example.com";
        let payload = r#"{"type":"CreateEntity","noun":"Order","id":"ord-1"}"#;
        assert!(!verify_signature(sender, payload, "deadbeefdeadbeef"),
            "bogus signature must fail verification");
        assert!(!verify_signature(sender, payload, ""),
            "empty signature must fail verification");
    }

    #[test]
    fn tampered_payload_fails() {
        // Attacker intercepts sender+sig and rewrites the payload.
        let sender = "alice@example.com";
        let payload_original = r#"{"noun":"Order","id":"ord-1"}"#;
        let payload_tampered = r#"{"noun":"Order","id":"ord-2"}"#;
        let sig = sign(sender, payload_original);
        assert!(!verify_signature(sender, payload_tampered, &sig),
            "signature over ord-1 must not verify against ord-2");
    }

    #[test]
    fn tampered_sender_fails() {
        // Attacker replays the message under a different sender identity.
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
        // Boundary confusion check: "ab" + "c" must differ from "a" + "bc".
        let s1 = sign("ab", "c");
        let s2 = sign("a", "bc");
        assert_ne!(s1, s2,
            "SEP between sender and payload must prevent field confusion");
    }
}
