// crates/arest/tests/e3_integration.rs
//
// Integration tests for E3 / #305 Citation-fact provenance.
// Sits in its own file (not properties.rs) because properties.rs has
// stale references to removed APIs that block `cargo test --test
// properties`. These tests run via `cargo test --test e3_integration`
// and exercise the public ast / engine surface only — no internal
// IR shapes.

use arest::ast;
use arest::compile::compile_to_defs_state;
use arest::parse_forml2::parse_to_state;

/// Helper — build a compiled state with federation defs from a
/// minimal readings fragment. Keeps the federation-domain setup
/// out of every test body.
fn federation_state() -> ast::Object {
    let readings = r#"
## Entity Types

User(.Email) is an entity type.
External System(.Name) is an entity type.
Noun(.id) is an entity type.
URL is a value type.
Header is a value type.
URI is a value type.

## Fact Types

External System has URL.
  Each External System has exactly one URL.
External System has Header.
  Each External System has at most one Header.
Noun is backed by External System.
  Each Noun is backed by at most one External System.
Noun has URI.
  Each Noun has at most one URI.
User has Email.
User has Name.

## Instance Facts

External System 'auth.vin' has URL 'https://auth.vin'.
External System 'auth.vin' has Header 'Authorization'.
Noun 'User' is backed by External System 'auth.vin'.
Noun 'User' has URI '/users'.
"#;
    let state = parse_to_state(readings).expect("parse should succeed");
    let defs = compile_to_defs_state(&state);
    ast::defs_to_state(&defs, &state)
}

/// The Authority Type enum in readings/instances.md must cover the
/// E3 provenance origins. A file-content check instead of a parsed
/// metamodel query because the latter requires a working compile
/// pipeline that is noisier to set up for a two-string assertion.
#[test]
fn authority_type_enum_includes_runtime_function_and_federated_fetch() {
    let instances = include_str!("../../../readings/instances.md");
    let enum_line = instances.lines()
        .find(|l| l.contains("possible values of Authority Type"))
        .expect("Authority Type enum declaration should exist in instances.md");
    assert!(enum_line.contains("'Runtime-Function'"),
        "Authority Type enum should declare 'Runtime-Function'; line: {enum_line}");
    assert!(enum_line.contains("'Federated-Fetch'"),
        "Authority Type enum should declare 'Federated-Fetch'; line: {enum_line}");
}

/// Compile emits BOTH populate:{noun} (compile-time config as
/// Func::constant) AND populate_fetch:{noun} (dispatchable
/// Func::Platform name) for every backed noun. The Platform-named
/// def gives engine-side apply a handle — a host can install a sync
/// callback via install_platform_fn("populate_fetch:<noun>", …) that
/// reads from a pre-staged cache cell, emits Citation, and returns
/// facts, so Rust derivations referencing the name trigger the
/// fetch path at apply time.
#[test]
fn compile_emits_populate_fetch_as_platform_dispatch() {
    let readings = r#"
# Mini Federation

## Entity Types

User(.Email) is an entity type.
External System(.Name) is an entity type.
Noun(.id) is an entity type.
URL is a value type.
Header is a value type.
Prefix is a value type.
URI is a value type.

## Fact Types

External System has URL.
  Each External System has exactly one URL.
External System has Header.
  Each External System has at most one Header.
Noun is backed by External System.
  Each Noun is backed by at most one External System.
Noun has URI.
  Each Noun has at most one URI.

## Instance Facts

External System 'auth.vin' has URL 'https://auth.vin'.
External System 'auth.vin' has Header 'Authorization'.
Noun 'User' is backed by External System 'auth.vin'.
Noun 'User' has URI '/users'.
"#;

    let state = parse_to_state(readings).expect("parse should succeed");
    let defs = compile_to_defs_state(&state);
    let defs_map: std::collections::HashMap<String, &ast::Func> =
        defs.iter().map(|(k, v)| (k.clone(), v)).collect();

    // Existing behaviour: populate:{noun} is compile-time config.
    assert!(defs_map.contains_key("populate:User"),
        "populate:User must exist (compile-time config)");

    // New behaviour: populate_fetch:{noun} is a dispatchable Platform name.
    let fetch_def = defs_map.get("populate_fetch:User")
        .expect("populate_fetch:User must be emitted for every backed noun");
    match fetch_def {
        ast::Func::Platform(name) => {
            assert_eq!(name, "populate_fetch:User",
                "populate_fetch:User body must be Func::Platform(populate_fetch:User) so apply_platform can dispatch; got Platform({name})");
        }
        other => panic!("populate_fetch:User must be a Func::Platform, got {other:?}"),
    }
}

/// RMAP / cell routing (#305 #14 verification): federated facts
/// pushed via `ingest_federated_facts` land in their declared FT
/// cells (Noun_verb_Role) — the same addressing the compile pipeline
/// and the command path (create / update / transition) use. A query
/// targeting the FT id retrieves them.
#[test]
fn federated_facts_land_in_standard_ft_cells_after_ingest() {
    let d = federation_state();
    let facts = vec![(
        "User_has_Email".to_string(),
        vec![
            ("User".to_string(), "u-1".to_string()),
            ("Email".to_string(), "alice@x.com".to_string()),
        ],
    )];
    let (_, d2) = ast::ingest_federated_facts(
        "auth.vin",
        "https://auth.vin/users",
        "2026-04-20T12:00:00Z",
        &facts,
        &d,
    );
    let cell = ast::fetch("User_has_Email", &d2);
    let entries = cell.as_seq().map(|s| s.to_vec()).unwrap_or_default();
    let matched_email = entries.iter()
        .find(|f| ast::binding(f, "User") == Some("u-1"))
        .and_then(|f| ast::binding(f, "Email"));
    assert_eq!(matched_email, Some("alice@x.com"),
        "federated fact must land in User_has_Email cell under standard RMAP-shaped addressing");
}

/// Derivation firing over federated facts (#305 #15 verification):
/// forward chaining runs at create-time inside command::create_via_defs,
/// NOT inside ingest_federated_facts. This test documents that contract:
/// post-ingest, downstream derivation cells are NOT auto-populated from
/// the new federated facts. Callers who need forward chaining must
/// trigger it (e.g., via an explicit apply of a derivation rule or a
/// follow-up create command). The paper's "facts enter P without
/// distinguishing" invariant holds at query time — any later rho-
/// application over P sees the federated facts — but the lfp loop is
/// not re-run automatically.
#[test]
fn federated_ingest_does_not_retrigger_forward_chain() {
    let d = federation_state();
    // Count derivation cells before ingest.
    let derivation_cell_count_before = ast::cells_iter(&d).iter()
        .filter(|(name, _)| name.starts_with("derivation:"))
        .count();

    // Ingest a user fact.
    let facts = vec![(
        "User_has_Email".to_string(),
        vec![
            ("User".to_string(), "u-1".to_string()),
            ("Email".to_string(), "alice@x.com".to_string()),
        ],
    )];
    let (_, d2) = ast::ingest_federated_facts(
        "auth.vin",
        "https://auth.vin/users",
        "2026-04-20T12:00:00Z",
        &facts,
        &d,
    );

    // Derivation *defs* (rules) are compile output; they don't change on
    // ingest. This assertion is the negative guard: ingesting a fact
    // does not add or remove any derivation rules, and does not trigger
    // their evaluation. Consumers who want forward chaining must
    // invoke it explicitly.
    let derivation_cell_count_after = ast::cells_iter(&d2).iter()
        .filter(|(name, _)| name.starts_with("derivation:"))
        .count();
    assert_eq!(derivation_cell_count_before, derivation_cell_count_after,
        "ingest must not alter the derivation-rule space");

    // The fact IS in P — so a rho-application over P that runs later
    // will see it. This is the paper's uniformity guarantee: federated
    // facts are not distinguished from local facts at query time.
    let cell = ast::fetch("User_has_Email", &d2);
    assert!(cell.as_seq().is_some_and(|s| !s.is_empty()),
        "the federated fact is still in P; later rho-apply sees it");
}

/// Smoke-test the end-to-end register → invoke → cite flow at the
/// integration boundary (not the inline ast::tests module). Drives
/// only the public API.
#[test]
fn runtime_fn_registration_plus_citation_emission_end_to_end() {
    // 1. Runtime registers a name and installs a body.
    let d = ast::register_runtime_fn(
        "e3_integ_greet",
        ast::Func::Platform("e3_integ_greet".to_string()),
        &ast::Object::phi(),
    );
    ast::install_platform_fn(
        "e3_integ_greet",
        arest::sync::Arc::new(|x: &ast::Object, _d: &ast::Object| {
            ast::Object::atom(&format!("hi {}", x.as_atom().unwrap_or("stranger")))
        }),
    );

    // 2. Apply dispatches to the installed callback.
    let out = ast::apply(
        &ast::Func::Def("e3_integ_greet".to_string()),
        &ast::Object::atom("world"),
        &d,
    );
    ast::uninstall_platform_fn("e3_integ_greet");
    assert_eq!(out, ast::Object::atom("hi world"));

    // 3. Citation for the call lands in P via emit_citation_fact.
    let (cite_id, d_with_cite) = ast::emit_citation_fact(
        "platform:e3_integ_greet",
        "Runtime-Function",
        "2026-04-20T12:00:00Z",
        None,
        &d,
    );
    let text_cell = ast::fetch("Citation_has_Text", &d_with_cite);
    let text_facts = text_cell.as_seq().map(|s| s.to_vec()).unwrap_or_default();
    assert_eq!(text_facts.len(), 1,
        "Citation_has_Text must have exactly one fact (alethic)");
    let text_binding = ast::binding(&text_facts[0], "Text").unwrap_or("");
    assert!(text_binding.contains("platform:e3_integ_greet"),
        "auto-text should mention the Platform name: {text_binding}");
    assert!(cite_id.starts_with("cite:"),
        "cite id should follow the 'cite:' scheme: {cite_id}");
}
