// crates/arest/tests/parser_fuzz.rs
//
// Property-fuzz harness for the FORML 2 stage-1 + stage-2 parser (#670).
//
// Pins one invariant: `parse_to_state(&s)` MUST NOT PANIC on any string
// input. It may return `Err`, it may produce a sparse / nonsensical
// state, but it must never reach an `unwrap`, slice OOB, integer
// overflow, or similar panic path. The same contract extends to the
// downstream compile step: any state that the parser produces must
// drive `compile::compile_to_defs_state` to completion without panic
// — otherwise a malformed reading from a tenant could crash the
// engine mid-`load_reading`.
//
// Mirrors the pattern from `cell_aead_fuzz` (#669): proptest generates
// inputs, the body wraps each call in `catch_unwind` so a panic shows
// up as a property failure with a shrunk counterexample instead of
// killing the whole test binary.
//
// ## Why this matters for shipping
//
// Support.auto.dev (the first production AREST app) accepts user-pasted
// readings via `system::load_reading`. Every entry point — UI paste, MCP
// `load_reading` verb, CLI `arest reload <file>` — funnels through
// `parse_to_state`. A panic on adversarial input is a denial-of-service
// at minimum and an information-disclosure surface (Rust panic messages
// can leak stack details) at worst. The fuzz harness is the floor: a
// passing run says "no parser panic on N kilo-cases of randomised
// input"; failures shrink to a minimal reproducer that the parser team
// can fix.
//
// ## Cap on input size
//
// 2 KiB max keeps each case sub-second on the proptest runner. The
// stage-2 parser is roughly linear in input length; longer inputs
// would shift the harness from "parser correctness" to "parser
// performance", which is a different property out of scope here.
//
// ## Composition coverage
//
// Three strategies feed the same `must_not_panic` body:
//
//   1. `arb_garbage`         — pure random bytes, sanitised to UTF-8.
//   2. `arb_forml_fragments` — concatenations of valid-ish FORML 2
//                              tokens (noun decls, fact-type readings,
//                              constraints, instance facts) interleaved
//                              with garbage. Hits more of the parser's
//                              tagged paths than pure random.
//   3. `arb_truncated`       — valid FORML 2 readings cut at random
//                              byte offsets. Exercises the "input
//                              ends mid-token / mid-derivation" paths
//                              that bare random rarely hits.

use std::panic::{catch_unwind, AssertUnwindSafe};

use arest::parse_forml2::parse_to_state;
use arest::compile::compile_to_defs_state;

use proptest::prelude::*;

// ── Strategies ──────────────────────────────────────────────────────

/// Pure random ASCII-ish bytes. Sanitised to valid UTF-8 by restricting
/// the character class to printable ASCII plus newline / tab.
fn arb_garbage() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        proptest::char::range('\t', '~'),
        0..2048,
    ).prop_map(|cs| cs.into_iter().collect())
}

/// Concatenations of valid-ish FORML 2 fragments interleaved with
/// random separators. Higher density of parser-recognised tokens
/// than `arb_garbage`, so the fuzzer hits more of the tagged
/// classifier / translator paths.
fn arb_forml_fragments() -> impl Strategy<Value = String> {
    let fragments = prop_oneof![
        // Noun declarations
        Just("Outbound Email(.id) is an entity type.\n".to_string()),
        Just("User(.id) is an entity type.\n".to_string()),
        Just("Status is a value type.\n".to_string()),
        // Subtype declarations
        Just("Agent is a subtype of User.\n".to_string()),
        // Fact type readings
        Just("User has Email.\n".to_string()),
        Just("Outbound Email is sent.\n".to_string()),
        Just("User approves Outbound Email.\n".to_string()),
        // Cardinality constraints
        Just("Each User has at most one Email.\n".to_string()),
        Just("For each Email, exactly one User has that Email.\n".to_string()),
        // Deontic
        Just("It is forbidden that some Outbound Email is sent.\n".to_string()),
        // Instance facts
        Just("State Machine Definition 'Order' is for Noun 'Order'.\n".to_string()),
        // Markdown headings
        Just("# Domain\n".to_string()),
        Just("## Entity Types\n".to_string()),
        // Random text
        "[a-zA-Z .,'\\n]{0,40}".prop_map(|s: String| s),
    ];
    proptest::collection::vec(fragments, 0..16)
        .prop_map(|parts| parts.join(""))
}

/// A valid-ish reading body, then truncated at a random byte offset.
/// Hits the "input ends mid-token / mid-derivation / mid-quoted-string"
/// paths that pure random input rarely surfaces.
fn arb_truncated() -> impl Strategy<Value = String> {
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
It is forbidden that some Outbound Email is sent and some User approves that Outbound Email and that User is Agent.\n";
    (0..=full.len()).prop_map(move |n| full[..n].to_string())
}

// ── Properties ──────────────────────────────────────────────────────

/// Wrap `parse_to_state` + `compile_to_defs_state` in `catch_unwind`
/// and return Ok(()) iff neither panicked. Either may return `Err` /
/// produce a sparse state — that's fine. Only a `panic!` (any kind:
/// `unwrap` on None, slice OOB, integer overflow with debug overflow
/// checks, infinite recursion stack overflow caught by stacker, …)
/// counts as a failure.
fn must_not_panic(input: &str) -> Result<(), String> {
    let parse_result = catch_unwind(AssertUnwindSafe(|| parse_to_state(input)));
    let state = match parse_result {
        Ok(Ok(state)) => state,
        Ok(Err(_)) => return Ok(()),  // Error return is a-OK
        Err(payload) => {
            return Err(format!(
                "parse_to_state panicked on input {input:?}: {}",
                downcast_panic(&payload),
            ));
        }
    };
    // Compile must also stay panic-free on whatever state the parser
    // produced. The compiler's downstream consumers (validate,
    // populate_imports, transitions:{noun}) don't run in this fuzz
    // because their inputs depend on more than just the readings;
    // compile_to_defs_state is the gate that catches every panic the
    // parser-emitted state could trigger downstream.
    let compile_result = catch_unwind(AssertUnwindSafe(|| compile_to_defs_state(&state)));
    match compile_result {
        Ok(_defs) => Ok(()),
        Err(payload) => Err(format!(
            "compile_to_defs_state panicked on input {input:?}: {}",
            downcast_panic(&payload),
        )),
    }
}

/// Best-effort panic-payload stringifier. `catch_unwind` returns a
/// `Box<dyn Any>`; the standard `panic!` payload is either `&str` or
/// `String`, so we try both before falling back to "<opaque>".
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
    /// Pure random ASCII-ish input — the broadest fuzz sweep. Most
    /// cases will be Err returns from the parser; the property is just
    /// that none of them panic.
    #[test]
    fn parse_garbage_never_panics(s in arb_garbage()) {
        must_not_panic(&s).map_err(|e| TestCaseError::fail(e))?;
    }

    /// Higher-signal: pre-formed FORML 2 fragments with random
    /// interleaving. Catches panics in the classifier / translator /
    /// rule-resolution paths that pure random input rarely reaches.
    #[test]
    fn parse_forml_fragments_never_panics(s in arb_forml_fragments()) {
        must_not_panic(&s).map_err(|e| TestCaseError::fail(e))?;
    }

    /// Truncated valid readings — catches mid-token / mid-derivation /
    /// mid-quoted-string handling that the other strategies skip.
    #[test]
    fn parse_truncated_readings_never_panics(s in arb_truncated()) {
        must_not_panic(&s).map_err(|e| TestCaseError::fail(e))?;
    }
}
