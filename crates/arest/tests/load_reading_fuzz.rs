// crates/arest/tests/load_reading_fuzz.rs
//
// Property-fuzz harness for `load_reading_core::load_reading` (#707 / Audit P1-A1).
//
// `load_reading` is the headline tenant-mutation surface: every UI
// paste, every `system::load_reading` MCP verb, every `arest reload
// <file>` CLI invocation, and every Cluster-3 follower replay funnels
// through it. Unlike `parser_fuzz` (which covers `parse_to_state` +
// `compile_to_defs_state` in isolation), this harness exercises the
// composed pipeline:
//
//   parse → merge → recompile → validate_loaded_state → write_manifest
//
// — the same path `apply_load_readings` (#705 / Audit D3) drives, plus
// the per-load deontic gate (#559 / DynRdg-5) and the manifest
// versioning step (#558).
//
// Pin: `load_reading(state, name, body, policy)` MUST NOT PANIC for
// any (name, body) pair, regardless of the input state. Returning
// `Err(LoadError::*)` is fine; producing an empty diff is fine; what
// is not fine is reaching an `unwrap`, slice OOB, integer overflow,
// or stack overflow that crashes the engine mid-load. A tenant that
// can panic the engine via the load surface is a denial-of-service
// vector at minimum.
//
// Mirrors `parser_fuzz` (#670): proptest generates inputs, the body
// wraps each call in `catch_unwind` so a panic surfaces as a property
// failure with a shrunk counterexample.
//
// ## Cap on input size
//
// 4 KiB max per body (audit's #707 ceiling). Names are clamped to 256
// bytes — the same `MAX_NAME_LEN` the on-disk persist record enforces
// (#708). Larger bodies shift the harness from "load correctness" to
// "load performance" — out of scope here.
//
// ## Composition coverage
//
// Three strategies feed the same `must_not_panic` body:
//
//   1. `arb_garbage_body` — pure random ASCII-ish bytes for the body.
//   2. `arb_forml_body`   — concatenations of valid-ish FORML 2
//                           fragments interleaved with garbage.
//   3. `arb_truncated_body` — a valid reading cut at random byte
//                             offsets, exercising mid-token /
//                             mid-derivation parser tail paths.

use std::panic::{catch_unwind, AssertUnwindSafe};

use arest::ast::Object;
use arest::load_reading_core::{load_reading, LoadReadingPolicy};

use proptest::prelude::*;

// ── Strategies ──────────────────────────────────────────────────────

/// Reading name. Clamped to `MAX_NAME_LEN = 256` ASCII-printable
/// chars — the on-disk persist surface caps name length here too
/// (#708 / Audit P2), so we mirror that ceiling.
fn arb_name() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_.-]{0,256}".prop_map(|s: String| s)
}

/// Pure random body — printable ASCII + tab + newline, capped at
/// 4 KiB. Most cases will round-trip to `Err(LoadError::ParseError)`;
/// the property is only no-panic.
fn arb_garbage_body() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        proptest::char::range('\t', '~'),
        0..4096,
    ).prop_map(|cs| cs.into_iter().collect())
}

/// Higher-signal body: pre-formed FORML 2 fragments interleaved with
/// random text. Hits more of the parser + classifier + validate paths
/// than pure random input.
fn arb_forml_body() -> impl Strategy<Value = String> {
    let fragments = prop_oneof![
        // Noun declarations
        Just("Outbound Email(.id) is an entity type.\n".to_string()),
        Just("User(.id) is an entity type.\n".to_string()),
        Just("Status is a value type.\n".to_string()),
        // Subtypes
        Just("Agent is a subtype of User.\n".to_string()),
        // Fact types
        Just("User has Email.\n".to_string()),
        Just("Outbound Email is sent.\n".to_string()),
        Just("User approves Outbound Email.\n".to_string()),
        // Constraints
        Just("Each User has at most one Email.\n".to_string()),
        Just("For each Email, exactly one User has that Email.\n".to_string()),
        // Deontic
        Just("It is forbidden that some Outbound Email is sent.\n".to_string()),
        // Instance facts (richer downstream coverage)
        Just("State Machine Definition 'Order' is for Noun 'Order'.\n".to_string()),
        // Markdown structure
        Just("# Domain\n".to_string()),
        Just("## Entity Types\n".to_string()),
        // Random text noise
        "[a-zA-Z .,'\\n]{0,40}".prop_map(|s: String| s),
    ];
    proptest::collection::vec(fragments, 0..16)
        .prop_map(|parts| parts.join(""))
}

/// Valid reading body, truncated at a random byte offset. Exercises
/// the "input ends mid-token / mid-quoted-string / mid-derivation"
/// paths that the other strategies skip.
fn arb_truncated_body() -> impl Strategy<Value = String> {
    let full = "# Outbound Email\n\
\n\
## Entity Types\n\
Outbound Email(.id) is an entity type.\n\
User(.id) is an entity type.\n\
\n\
## Fact Types\n\
User approves Outbound Email.\n\
Outbound Email is sent.\n\
\n\
## Constraints\n\
Each Outbound Email is sent at most once.\n\
\n\
## Deontic Constraints\n\
It is forbidden that some Outbound Email is sent and some User approves that Outbound Email.\n";
    (0..=full.len()).prop_map(move |n| full[..n].to_string())
}

// ── Properties ──────────────────────────────────────────────────────

/// Wrap `load_reading` in `catch_unwind` and return Ok(()) iff it
/// didn't panic. `Err(LoadError::*)` is a healthy outcome — only an
/// actual panic counts as a fuzz failure.
fn must_not_panic(name: &str, body: &str) -> Result<(), String> {
    // Empty starting state — minimal coverage of the merge step.
    // (Loading against the bundled metamodel would exercise more of
    // the validate dispatch, but it's also ~50× larger and would
    // dominate fuzz wall-time per case.)
    let state = Object::phi();
    let result = catch_unwind(AssertUnwindSafe(|| {
        load_reading(&state, name, body, LoadReadingPolicy::AllowAll)
    }));
    match result {
        // Outcome doesn't matter — Err is fine, Ok is fine, what
        // matters is that we didn't reach a panic path.
        Ok(_) => Ok(()),
        Err(payload) => Err(format!(
            "load_reading panicked on (name={name:?}, body={body:?}): {}",
            downcast_panic(&payload),
        )),
    }
}

/// Best-effort panic-payload stringifier. Standard `panic!` payloads
/// are either `&str` or `String`; fall back to "<opaque>" for the rest.
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
    /// Pure random body with a sanitised reading name. Broadest fuzz
    /// sweep — most cases bounce off `LoadError::ParseError`.
    #[test]
    fn load_garbage_body_never_panics(
        name in arb_name(),
        body in arb_garbage_body(),
    ) {
        must_not_panic(&name, &body).map_err(TestCaseError::fail)?;
    }

    /// FORML 2 fragments interleaved with noise. Higher chance of
    /// reaching merge + recompile + validate paths than pure random.
    #[test]
    fn load_forml_body_never_panics(
        name in arb_name(),
        body in arb_forml_body(),
    ) {
        must_not_panic(&name, &body).map_err(TestCaseError::fail)?;
    }

    /// Valid reading truncated at random offsets — the mid-token tail
    /// surface that pure random input rarely hits.
    #[test]
    fn load_truncated_body_never_panics(
        name in arb_name(),
        body in arb_truncated_body(),
    ) {
        must_not_panic(&name, &body).map_err(TestCaseError::fail)?;
    }
}
