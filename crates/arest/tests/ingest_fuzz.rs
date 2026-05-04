// crates/arest/tests/ingest_fuzz.rs
//
// Property-fuzz harness for `ingest::forward_chain_to_json` (#707 /
// Audit P1-A2). The webhook ingest surface is by-design adversarial:
// the JSON payload comes from external HTTP callers (Stripe events,
// GitHub events, support inbox, arbitrary tenant webhook senders).
// A panic here would be a denial-of-service against the worker /
// kernel HTTP path that wraps it.
//
// Pin: `forward_chain_to_json(state, input_json)` MUST NOT PANIC on
// any string input. It may return `{"derived": {}}` for unrecognised
// input — the function explicitly takes the "best-effort: if input
// is malformed, run derivations on an empty input" path — but never
// reach an `unwrap` / OOB / overflow panic.
//
// Mirrors `parser_fuzz` (#670) and `load_reading_fuzz` (#707-A1).
//
// ## Cap on input size
//
// 4 KiB max (audit ceiling). Longer inputs would shift the harness
// from "ingest correctness" to "ingest performance".
//
// ## Composition coverage
//
// Three strategies feed the same `must_not_panic` body:
//
//   1. `arb_garbage_json` — pure random ASCII-ish bytes.
//   2. `arb_wonky_json`   — JSON-shaped fragments (braces, commas,
//                           strings, numbers) interleaved with noise.
//                           Hits more of the JSON tokenizer's branches.
//   3. `arb_truncated_json` — a real-shaped webhook payload cut at
//                             a random byte offset.

use std::panic::{catch_unwind, AssertUnwindSafe};

use arest::ast::Object;
use arest::ingest::forward_chain_to_json;

use proptest::prelude::*;

// ── Strategies ──────────────────────────────────────────────────────

/// Pure random printable ASCII + tab + newline, capped at 4 KiB.
/// Most cases miss the JSON `{` opener and bounce immediately to the
/// best-effort empty-input branch; the property is only no-panic.
fn arb_garbage_json() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        proptest::char::range('\t', '~'),
        0..4096,
    ).prop_map(|cs| cs.into_iter().collect())
}

/// JSON-token salad. Higher chance of reaching the parser's typed
/// dispatch arms (number / string / object / array / true / false /
/// null) than truly-random bytes.
fn arb_wonky_json() -> impl Strategy<Value = String> {
    let tokens = prop_oneof![
        Just("{".to_string()),
        Just("}".to_string()),
        Just("[".to_string()),
        Just("]".to_string()),
        Just(":".to_string()),
        Just(",".to_string()),
        Just("\"key\"".to_string()),
        Just("\"\"".to_string()),
        Just("\"deeply.nested.path\"".to_string()),
        Just("123".to_string()),
        Just("-42".to_string()),
        Just("3.14e-9".to_string()),
        Just("true".to_string()),
        Just("false".to_string()),
        Just("null".to_string()),
        Just("\"\\u0000\"".to_string()),
        Just("\"\\u00ff\"".to_string()),
        Just("\"\\\"\"".to_string()),
        Just("\"\\n\\t\\r\"".to_string()),
        // Common webhook-shaped fragments
        Just("\"event\":".to_string()),
        Just("\"factType\":".to_string()),
        Just("\"bindings\":".to_string()),
        // Random whitespace + filler
        "[ \\t\\n]{0,8}".prop_map(|s: String| s),
        "[a-z0-9_-]{0,20}".prop_map(|s: String| s),
    ];
    proptest::collection::vec(tokens, 0..64)
        .prop_map(|parts| parts.join(""))
}

/// A real-shaped webhook payload truncated at a random byte offset.
/// Hits the "input ends mid-string / mid-number / mid-escape" tail
/// paths the other strategies skip.
fn arb_truncated_json() -> impl Strategy<Value = String> {
    let full = "{\
\"event\":\"order.placed\",\
\"factType\":\"Customer places Order\",\
\"bindings\":{\
\"Customer\":\"cust-1\",\
\"Order\":\"ord-42\",\
\"placedAt\":\"2026-05-04T12:00:00Z\"\
}\
}";
    (0..=full.len()).prop_map(move |n| full[..n].to_string())
}

// ── Properties ──────────────────────────────────────────────────────

fn must_not_panic(input: &str) -> Result<(), String> {
    // Empty starting state — the function explicitly tolerates this
    // and falls back to the SM-from-cells derivation path. Loading
    // the bundled metamodel would multiply per-case cost ~50× and
    // wouldn't change the panic surface (which lives in the JSON
    // parser + binding resolver, both of which run regardless).
    let state = Object::phi();
    let result = catch_unwind(AssertUnwindSafe(|| {
        forward_chain_to_json(&state, input)
    }));
    match result {
        Ok(_json) => Ok(()),
        Err(payload) => Err(format!(
            "forward_chain_to_json panicked on input ({} bytes): {}",
            input.len(),
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
    /// Pure random JSON-ish input. Most cases bounce to the empty-
    /// input fallback; property is only no-panic.
    #[test]
    fn ingest_garbage_never_panics(s in arb_garbage_json()) {
        must_not_panic(&s).map_err(TestCaseError::fail)?;
    }

    /// JSON-token salad with realistic webhook fragment text.
    /// Higher chance of reaching the typed dispatch arms.
    #[test]
    fn ingest_wonky_never_panics(s in arb_wonky_json()) {
        must_not_panic(&s).map_err(TestCaseError::fail)?;
    }

    /// Real webhook payload truncated at random offsets — covers
    /// mid-string / mid-number / mid-escape tail paths.
    #[test]
    fn ingest_truncated_never_panics(s in arb_truncated_json()) {
        must_not_panic(&s).map_err(TestCaseError::fail)?;
    }
}
