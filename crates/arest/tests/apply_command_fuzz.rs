// crates/arest/tests/apply_command_fuzz.rs
//
// Property-fuzz harness for `command::apply_command_defs` (#707 /
// Audit P1-A5). The Command dispatch pipeline is reached by every
// SYSTEM verb invocation: HTTP create / update / transition / query
// on worker + kernel, MCP `apply` verb, CLI `arest <verb> <input>`,
// and the cluster follower's replicated mutations. Inputs cross a
// JSON deserialisation boundary (serde-derived) and then hit the
// dispatch arms with attacker-controllable noun / domain / id /
// field-map content.
//
// Pin: `apply_command_defs(d, command, state)` MUST NOT PANIC for
// any well-typed Command + any (d, state) pair. Returning a
// rejected CommandResult is fine; reaching an `unwrap` / OOB /
// overflow is a fuzz failure.
//
// Mirrors `parser_fuzz` (#670) shape with proptest + catch_unwind.
//
// ## Cap on input size
//
// Fields capped at 64-byte values, 8-entry maps; noun / domain /
// id strings capped at 32 bytes. These dominate per-case cost
// (each command runs the full create / transition / update
// pipeline including diff computation); larger inputs would shift
// the harness from "dispatch correctness" to "perf".
//
// ## Coverage
//
// Three command families, each its own proptest:
//
//   1. CreateEntity — the headline write surface.
//   2. UpdateEntity — the second write surface; field-map shape
//      identical to create's.
//   3. Transition   — SM dispatch; the noun + entity_id + event
//      trio is the panic-prone path (looking up a missing SM
//      definition, missing transition, etc.).
//
// Query / LoadReadings / LoadReading / UnloadReading / ReloadReading
// are covered by the existing `load_reading_fuzz` (#707-A1) and the
// per-handler unit tests; not duplicated here.

use std::panic::{catch_unwind, AssertUnwindSafe};

use arest::ast::Object;
use arest::command::{apply_command_defs, Command};

use proptest::prelude::*;

// ── Strategies ──────────────────────────────────────────────────────

/// Short ASCII-printable string. Used for noun / domain / id /
/// field-key / field-value generation. Restricted to a glyph class
/// that survives JSON serialisation without escaping for clarity in
/// shrunk counterexamples.
fn arb_short_str(max: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(
        proptest::char::range(' ', '~'),
        0..max,
    ).prop_map(|cs| cs.into_iter().collect())
}

/// 0..8 entries of (key, value) string pairs. Mirrors the wire
/// shape of `CreateEntity::fields` / `UpdateEntity::fields`.
fn arb_field_map() -> impl Strategy<Value = hashbrown::HashMap<String, String>> {
    proptest::collection::vec(
        (arb_short_str(16), arb_short_str(64)),
        0..8,
    ).prop_map(|kvs| kvs.into_iter().collect())
}

/// Strategy for `Command::CreateEntity` with random but well-typed
/// content. `signature` is None — sig verification is its own
/// surface (sec_2_platform_fallback_audit + the dedicated unit tests).
fn arb_create() -> impl Strategy<Value = Command> {
    (
        arb_short_str(32),  // noun
        arb_short_str(32),  // domain
        proptest::option::of(arb_short_str(32)),  // id
        arb_field_map(),
        proptest::option::of(arb_short_str(32)),  // sender
    ).prop_map(|(noun, domain, id, fields, sender)| {
        Command::CreateEntity { noun, domain, id, fields, sender, signature: None }
    })
}

/// Strategy for `Command::UpdateEntity`. entity_id is required (not
/// Option) per the wire schema.
fn arb_update() -> impl Strategy<Value = Command> {
    (
        arb_short_str(32),  // noun
        arb_short_str(32),  // domain
        arb_short_str(32),  // entity_id
        arb_field_map(),
        proptest::option::of(arb_short_str(32)),  // sender
    ).prop_map(|(noun, domain, entity_id, fields, sender)| {
        Command::UpdateEntity { noun, domain, entity_id, fields, sender, signature: None }
    })
}

/// Strategy for `Command::Transition`. The (entity_id, event,
/// current_status) tuple drives the SM lookup — the most panic-
/// prone arm because state-machine definitions live in the def
/// graph and a missing one must degrade gracefully.
fn arb_transition() -> impl Strategy<Value = Command> {
    (
        arb_short_str(32),  // entity_id
        arb_short_str(32),  // event
        arb_short_str(32),  // domain
        proptest::option::of(arb_short_str(32)),  // current_status
        proptest::option::of(arb_short_str(32)),  // sender
    ).prop_map(|(entity_id, event, domain, current_status, sender)| {
        Command::Transition { entity_id, event, domain, current_status, sender, signature: None }
    })
}

// ── Properties ──────────────────────────────────────────────────────

fn must_not_panic(cmd: &Command) -> Result<(), String> {
    // Empty def + state. The dispatch arms should degrade gracefully
    // when the noun isn't in `d` (no constraint defs, no SM
    // transitions). Loading the bundled metamodel would multiply
    // per-case cost ~50× without changing the panic surface — the
    // create / update / transition handlers don't depend on which
    // FactType cells are populated, only on whether the lookup
    // returns something cleanly.
    let d = Object::phi();
    let state = Object::phi();
    let result = catch_unwind(AssertUnwindSafe(|| {
        apply_command_defs(&d, cmd, &state)
    }));
    match result {
        Ok(_cmd_result) => Ok(()),
        Err(payload) => Err(format!(
            "apply_command_defs panicked on command {cmd:?}: {}",
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
    /// CreateEntity over arbitrary noun / domain / id / field-map.
    /// The headline tenant write surface — every HTTP POST /api/
    /// entities lands here.
    #[test]
    fn create_never_panics(cmd in arb_create()) {
        must_not_panic(&cmd).map_err(TestCaseError::fail)?;
    }

    /// UpdateEntity over arbitrary noun / domain / entity_id /
    /// field-map. Same surface as create on the field-map side.
    #[test]
    fn update_never_panics(cmd in arb_update()) {
        must_not_panic(&cmd).map_err(TestCaseError::fail)?;
    }

    /// Transition over arbitrary entity_id / event / current_status.
    /// The most panic-prone arm because SM lookup may miss; the
    /// handler must surface that as a rejected result, not a panic.
    #[test]
    fn transition_never_panics(cmd in arb_transition()) {
        must_not_panic(&cmd).map_err(TestCaseError::fail)?;
    }
}
