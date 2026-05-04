// crates/arest/tests/freeze_thaw_fuzz.rs
//
// Property-fuzz harness for `freeze::thaw` and `freeze::thaw_sealed`
// (#707 / Audit P1-A3). The production storage path: every kernel
// checkpoint replay, every cluster follower receive (#706), and every
// DurableObject thaw on the worker reaches one of these two functions
// with bytes the engine didn't produce — corrupted on disk, tampered
// in transit, replayed across schema versions, etc. A panic here is
// either a denial-of-service (a malicious snapshot crashes a tenant on
// reload) or an information-disclosure (Rust panic messages can leak
// pointer / stack details).
//
// Pin: `thaw(bytes)` and `thaw_sealed(bytes, master, scope, domain,
// version)` MUST NOT PANIC for any byte input. They may return Err,
// they may return a degenerate Object, but reaching an `unwrap`,
// slice OOB, or integer overflow is a fuzz failure.
//
// Mirrors `parser_fuzz` (#670) and `cell_aead_fuzz` (#669).
//
// ## Cap on input size
//
// 4 KiB max (audit ceiling). The freeze format is roughly proportional
// to the source Object size; longer inputs would shift this from
// "thaw correctness" to "thaw performance" — out of scope.
//
// ## Composition coverage
//
// Three strategies share the same `must_not_panic` body for `thaw`:
//
//   1. `arb_garbage_bytes` — pure random bytes.
//   2. `arb_freeze_image_with_corruption` — a real freeze image with
//      a single byte flipped. Targets the framing / length-prefix
//      decode paths the truly-random sweep rarely hits cleanly.
//   3. `arb_truncated_freeze` — a real freeze image cut at a random
//      offset. Hits the "truncated header / mid-string" tails.
//
// `thaw_sealed` gets the garbage strategy only — a corrupted sealed
// blob almost-always trips the AEAD AAD check, but the framing parser
// runs first and is the panic surface that matters here.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

use arest::ast::{Object, fact_from_pairs, store};
use arest::freeze::{freeze, thaw, freeze_sealed, thaw_sealed};
use arest::entropy::{self, DeterministicSource};
use arest_foundation::cell_aead::TenantMasterKey;

use proptest::prelude::*;

/// Install a deterministic entropy source exactly once per process.
/// `freeze_sealed` draws its nonce from `arest::csprng`, which panics
/// if no source is installed — same setup pattern cell_aead_fuzz uses.
fn ensure_entropy_installed() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        entropy::install(Box::new(DeterministicSource::new([0x5Au8; 32])));
    });
}

// ── Strategies ──────────────────────────────────────────────────────

/// Pure random bytes capped at 4 KiB. Most cases hit the magic-check
/// or length-prefix bound check; the property is only no-panic.
fn arb_garbage_bytes() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..4096)
}

/// A representative freeze image: a small Map state with a Noun cell
/// and an InstanceFact cell. Used by `arb_freeze_image_with_corruption`
/// + `arb_truncated_freeze` as the seed bytes.
fn sample_freeze_image() -> Vec<u8> {
    let nouns = Object::seq(vec![fact_from_pairs(&[
        ("name", "Order"),
        ("objectType", "entity"),
    ])]);
    let facts = Object::seq(vec![fact_from_pairs(&[
        ("subjectNoun", "Order"),
        ("subjectValue", "ord-1"),
        ("fieldName", "Customer"),
        ("objectNoun", "Customer"),
        ("objectValue", "cust-1"),
    ])]);
    let state = store("Noun", nouns, &Object::phi());
    let state = store("InstanceFact", facts, &state);
    freeze(&state)
}

/// A real freeze image with one byte XOR-flipped at a random offset.
/// Catches panics in length-prefix paths the truly-random sweep
/// rarely reaches with valid magic.
fn arb_freeze_image_with_corruption() -> impl Strategy<Value = Vec<u8>> {
    let image = sample_freeze_image();
    let len = image.len();
    (0..len, any::<u8>()).prop_map(move |(idx, mask)| {
        let mut bytes = image.clone();
        bytes[idx] ^= mask.max(1);  // ensure at least one bit flipped
        bytes
    })
}

/// A real freeze image truncated at a random byte offset. Hits the
/// mid-header / mid-cell-name / mid-payload tail paths.
fn arb_truncated_freeze() -> impl Strategy<Value = Vec<u8>> {
    let image = sample_freeze_image();
    let len = image.len();
    (0..=len).prop_map(move |n| image[..n].to_vec())
}

// ── Properties ──────────────────────────────────────────────────────

fn thaw_must_not_panic(bytes: &[u8]) -> Result<(), String> {
    let result = catch_unwind(AssertUnwindSafe(|| thaw(bytes)));
    match result {
        Ok(_) => Ok(()),
        Err(payload) => Err(format!(
            "thaw panicked on {} bytes: {}",
            bytes.len(),
            downcast_panic(&payload),
        )),
    }
}

fn thaw_sealed_must_not_panic(bytes: &[u8]) -> Result<(), String> {
    // Fixed master + scope + domain + version. Nearly every random
    // input will fail the AEAD AAD check; the property is only that
    // the framing parser doesn't panic before we get there.
    let master = TenantMasterKey::from_bytes([0xAB; 32]);
    let result = catch_unwind(AssertUnwindSafe(|| {
        thaw_sealed(bytes, &master, "arest", "test", 1)
    }));
    match result {
        Ok(_) => Ok(()),
        Err(payload) => Err(format!(
            "thaw_sealed panicked on {} bytes: {}",
            bytes.len(),
            downcast_panic(&payload),
        )),
    }
}

fn downcast_panic(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return s.to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "<opaque panic payload>".to_string()
}

proptest! {
    /// `thaw` on pure random bytes. Broadest sweep; most inputs miss
    /// the magic header and bounce immediately with `bad magic`.
    #[test]
    fn thaw_garbage_never_panics(bytes in arb_garbage_bytes()) {
        thaw_must_not_panic(&bytes).map_err(TestCaseError::fail)?;
    }

    /// `thaw` on a real image with a single bit flipped. Targets
    /// length-prefix decode panics the truly-random sweep rarely hits.
    #[test]
    fn thaw_corrupted_image_never_panics(bytes in arb_freeze_image_with_corruption()) {
        thaw_must_not_panic(&bytes).map_err(TestCaseError::fail)?;
    }

    /// `thaw` on a real image truncated at random offsets. Catches
    /// mid-header / mid-cell-name / mid-payload tail panics.
    #[test]
    fn thaw_truncated_image_never_panics(bytes in arb_truncated_freeze()) {
        thaw_must_not_panic(&bytes).map_err(TestCaseError::fail)?;
    }

    /// `thaw_sealed` on pure random bytes against a fixed master /
    /// scope / domain / version. Random framing almost-always trips
    /// the framing parser or the AEAD AAD check; the property is
    /// only that neither path panics.
    #[test]
    fn thaw_sealed_garbage_never_panics(bytes in arb_garbage_bytes()) {
        thaw_sealed_must_not_panic(&bytes).map_err(TestCaseError::fail)?;
    }

    /// Round-trip sanity: any state we freeze must thaw back to the
    /// same object. Asserts the production read/write contract holds
    /// across the proptest input space — a slim guard but pins the
    /// "freeze ∘ thaw = id" invariant the storage path depends on.
    #[test]
    fn freeze_then_thaw_roundtrips(seed in any::<u32>()) {
        // Synthesise a Map state whose contents vary with the seed.
        let nouns = Object::seq(vec![
            fact_from_pairs(&[("name", &format!("Noun{seed}")), ("objectType", "entity")]),
        ]);
        let state = store("Noun", nouns, &Object::phi());
        let bytes = freeze(&state);
        let back = thaw(&bytes).expect("freeze image must thaw");
        prop_assert_eq!(state, back);
    }

    /// Round-trip sanity for sealed blobs. Same shape as above, but
    /// across `freeze_sealed` / `thaw_sealed` so the AEAD path stays
    /// pinned too — corruption is covered by the garbage strategies
    /// above; this case asserts the happy path.
    #[test]
    fn freeze_sealed_then_thaw_sealed_roundtrips(seed in any::<u32>()) {
        ensure_entropy_installed();
        let master = TenantMasterKey::from_bytes([0xAB; 32]);
        let nouns = Object::seq(vec![
            fact_from_pairs(&[("name", &format!("Noun{seed}")), ("objectType", "entity")]),
        ]);
        let state = store("Noun", nouns, &Object::phi());
        let bytes = freeze_sealed(&state, &master, "arest", "test", 1);
        let back = thaw_sealed(&bytes, &master, "arest", "test", 1)
            .expect("sealed freeze must thaw with matching key/AAD");
        prop_assert_eq!(state, back);
    }
}
