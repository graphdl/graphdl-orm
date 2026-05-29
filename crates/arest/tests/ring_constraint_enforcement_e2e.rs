// crates/arest/tests/ring_constraint_enforcement_e2e.rs
//
// Regression pin for ORM 2 ring-constraint enforcement on the
// `Task blocks Task` self-relation declared in
// apps/tasks/readings/app.md:
//
//     Task blocks Task is irreflexive.
//     Task blocks Task is asymmetric.
//
// Why this test exists
// --------------------
// The operator asked whether the tasks ring constraints actually *run*
// ("tasks had ring constraint issues for a while"). Three things made
// that question impossible to answer with confidence beforehand:
//
//   * The `validate` MCP/CLI verb is BLIND to this. It evaluates the
//     compiled constraint Func against the *stored* population (which is
//     clean), so deliberately-injected violation facts never enter the
//     evaluated state — it reports `satisfied:true` for an explicit
//     self-block, which only re-confirms the stored 400 edges are clean.
//   * No integration test covered ring rejection at all — exactly the
//     blind spot that lets a ring regression ship unnoticed.
//   * `Task blocks Task` is the one ring relation in the live tasks DB,
//     so a silent miss here corrupts the real task-dependency graph.
//
// This test exercises the AUTHORITATIVE runtime path — the same one
// `eval_constraints_defs` / `create_via_defs` use: compile the reading,
// place violation facts in the population, apply each `constraint:` def
// to the encoded population context, and decode the violations.
//
// Contract pinned
// ---------------
//   1. irreflexive (IR) fires on a self-block `t1 blocks t1`;
//   2. asymmetric (AS) fires on a reciprocal pair `t2 blocks t3` + `t3 blocks t2`;
//   3. every emitted ring violation is alethic (decode_violation sets
//      `alethic:true`) — so `create_via_defs` hard-rejects (D' = D),
//      the smart-contract soundness contract: a violating write can
//      never land in a valid state;
//   4. a clean block graph yields ZERO ring violations — no
//      false-positive that would reject legitimate task dependencies
//      (over-enforcement is just as much a "ring constraint issue" as
//      under-enforcement).

use arest::ast;

/// Compile `src`, then evaluate every `constraint:` def against the
/// parsed population exactly as the runtime validate path does
/// (evaluate.rs::eval_constraints_defs): the population rides in the
/// eval context, the compiled def map is the apply environment.
///
/// Returns `(constraint_text, detail, alethic)` triples so the test
/// never has to name the `Violation` type across the crate boundary.
fn constraint_violations(src: &str) -> Vec<(String, String, bool)> {
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    let ctx = ast::encode_eval_context_state("", None, &state);

    defs.iter()
        .filter(|(name, _)| name.starts_with("constraint:"))
        .flat_map(|(_, func)| ast::decode_violations(&ast::apply(func, &ctx, &d)))
        .map(|v| (v.constraint_text, v.detail, v.alethic))
        .collect()
}

/// True when the violation's reading text or rendered detail mentions
/// `needle` (case-insensitive). IR detail templates render
/// "Irreflexive violation: …", AS render "Asymmetric violation: …";
/// the `constraint_text` carries the verbatim reading either way.
fn mentions(v: &(String, String, bool), needle: &str) -> bool {
    let n = needle.to_lowercase();
    v.0.to_lowercase().contains(&n) || v.1.to_lowercase().contains(&n)
}

#[test]
fn ring_constraints_fire_on_self_block_and_reciprocal_pair() {
    // t1→t1 violates irreflexive; t2↔t3 violates asymmetric.
    let src = "\
        Task(.id) is an entity type.\n\
        Task blocks Task.\n\
        Task blocks Task is irreflexive.\n\
        Task blocks Task is asymmetric.\n\
        Task 't1' blocks Task 't1'.\n\
        Task 't2' blocks Task 't3'.\n\
        Task 't3' blocks Task 't2'.\n\
    ";
    let v = constraint_violations(src);

    assert!(
        v.iter().any(|x| mentions(x, "irreflexive")),
        "irreflexive constraint must fire on the self-block t1→t1; \
         got violations: {:?}",
        v
    );
    assert!(
        v.iter().any(|x| mentions(x, "asymmetric")),
        "asymmetric constraint must fire on the reciprocal pair t2↔t3; \
         got violations: {:?}",
        v
    );
    assert!(
        v.iter().all(|x| x.2),
        "ring violations must be alethic so create_via_defs hard-rejects \
         (D'=D — a violating write can never land); got: {:?}",
        v
    );
}

#[test]
fn ring_constraints_pass_on_clean_block_graph() {
    // t1→t2, t2→t3, t1→t3: irreflexive, asymmetric, acyclic.
    // No self-loop, no reciprocal pair — must produce no ring violation.
    let src = "\
        Task(.id) is an entity type.\n\
        Task blocks Task.\n\
        Task blocks Task is irreflexive.\n\
        Task blocks Task is asymmetric.\n\
        Task 't1' blocks Task 't2'.\n\
        Task 't2' blocks Task 't3'.\n\
        Task 't1' blocks Task 't3'.\n\
    ";
    let ring: Vec<_> = constraint_violations(src)
        .into_iter()
        .filter(|x| mentions(x, "irreflexive") || mentions(x, "asymmetric"))
        .collect();

    assert!(
        ring.is_empty(),
        "a clean block graph must yield no ring violations (no \
         false-positive rejection of valid task dependencies); got: {:?}",
        ring
    );
}
