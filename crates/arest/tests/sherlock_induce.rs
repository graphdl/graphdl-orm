// crates/arest/tests/sherlock_induce.rs
//
// Acceptance test for AREST task #853 — Sherlock app fixture exercises
// the induce primitive against a tiny case (one Case + three Evidence
// + two candidate Hypotheses), asserting the top-ranked Hypothesis
// Candidate matches the intended explanation.
//
// "Readings as source code": all scoring/ranking lives in the Sherlock
// app's FORML2 readings (`reasoning.md` Scoring Rules + the test
// fixture's Plausibility-Mark assignments) — this Rust test only loads
// the readings, compiles, runs forward-chain LFP once, invokes
// `Func::Platform("induce")`, and asserts on the ordered result.
//
// The fixture lives at `apps/sherlock/readings/cases/test-locked-room.md`
// alongside the existing Doyle case files. The Scoring Rules + the
// `Hypothesis_has_Plausibility` FT live in
// `apps/sherlock/readings/reasoning.md`.

use arest::ast::{self, defs_to_state, Func, Object};
use arest::compile::compile_to_defs_state;
use arest::parse_forml2::parse_to_state_from;

// -- Sherlock app readings (schema) ---------------------------------
// We deliberately exclude `evidence.md` and `investigation.md` from the
// search state (see `merged_state` doc-comment). They're available as
// parser context if a future test wants to compose the full schema.
const CASES_MD:    &str = include_str!("../../../../apps/sherlock/readings/cases.md");
const CRIME_MD:    &str = include_str!("../../../../apps/sherlock/readings/crime.md");
const REASONING_MD: &str = include_str!("../../../../apps/sherlock/readings/reasoning.md");

// -- Test fixture ---------------------------------------------------
const TEST_LOCKED_ROOM_MD: &str =
    include_str!("../../../../apps/sherlock/readings/cases/test-locked-room.md");

// -- Bundled substrate vocabulary -----------------------------------
// `induction.md` declares Hypothesis Candidate / Confidence Score /
// Scoring Rule. `instances.md` declares Fact. Both are pulled in as
// parser context (so references resolve) but only `induction.md`'s
// schema cells land in the search state — `instances.md` carries
// Fact-belongs-to-Domain mandatory constraints that the Sherlock
// fixture would have to fully satisfy across every populated cell.
const INDUCTION_MD: &str = include_str!("../../../readings/core/induction.md");
const INSTANCES_MD: &str = include_str!("../../../readings/core/instances.md");

/// Fold the Sherlock schema + the test fixture into one merged state.
///
/// We deliberately AVOID `arest::metamodel_corpus()` here — the full
/// metamodel adds ~1500 baseline alethic violations from cells the
/// Sherlock app doesn't reference, which would make
/// `induce::candidate_passes_constraints` reject every candidate (the
/// gate is "no `validate` violation surfaces" — it doesn't subtract a
/// pre-existing baseline).
///
/// We also exclude `evidence.md` and `investigation.md` from the
/// search state:
///
/// * `investigation.md` — the Investigation SM forces every Case into
///   a Status. Without an explicit Status row in the fixture the
///   alethic gate fires `Each Case has some Status`.
/// * `evidence.md` — the Evidence Weight derivation rule emits a
///   literal-pin consequent the parser doesn't bind cleanly,
///   polluting the Evidence_has_Evidence_Weight cell with weight-less
///   rows that then trip the `Each Evidence has exactly one Evidence
///   Weight` mandatory constraint. The Evidence vocabulary the
///   fixture relies on (Evidence Source + Reliability + Evidence Weight)
///   is declared inline in the test fixture instead, with explicit
///   `Evidence has Evidence Weight 'Strong' / 'Weak'` rows so the gate
///   sees a fully populated state.
///
/// The fixture is still under `apps/sherlock/readings/cases/` per #853
/// — it's the Sherlock-app version of the engine's `induce::tests`
/// minimal-state pattern, plus a real Scoring Rule in `reasoning.md`.
fn merged_state() -> Object {
    // Parse-only context: lets references to `Fact`, `Hypothesis Candidate`,
    // `Confidence Score`, etc. resolve in subsequent reading parses.
    let mut context = Object::phi();
    for text in [INSTANCES_MD, INDUCTION_MD] {
        let parsed = parse_to_state_from(text, &context)
            .expect("substrate vocabulary parses");
        context = ast::merge_states(&context, &parsed);
    }

    // Search state: starts empty, accumulates ONLY Sherlock-defined cells.
    let mut state = Object::phi();
    for (name, text) in [
        ("crime.md",            CRIME_MD),
        ("cases.md",            CASES_MD),
        ("reasoning.md",        REASONING_MD),
        ("test-locked-room.md", TEST_LOCKED_ROOM_MD),
    ] {
        // Parse with the substrate context + accumulated Sherlock cells
        // so references to nouns from earlier readings + substrate
        // vocabulary resolve. Merge ONLY the parsed result into the
        // search state (context cells stay external).
        let merged_ctx = ast::merge_states(&context, &state);
        let parsed = parse_to_state_from(text, &merged_ctx)
            .unwrap_or_else(|e| panic!("parse {} failed: {}", name, e));
        state = ast::merge_states(&state, &parsed);
    }

    // Pull in induction.md's schema cells (FactType, Role, Constraint
    // for Hypothesis Candidate / Confidence Score) so the engine's
    // synthetic `Hypothesis_Candidate_has_hidden_<ObjectNoun>` projection
    // in `induce::score_candidate` lines up with declared FT shapes.
    // induction.md adds zero new mandatory constraints over the Sherlock
    // fixture's population so this is gate-safe.
    let induction_only = parse_to_state_from(INDUCTION_MD, &Object::phi())
        .expect("induction.md parses standalone");
    state = ast::merge_states(&state, &induction_only);
    state
}

/// Run forward-chain LFP on `d` against every `derivation:*` /
/// `derivation_strat2:*` def in `d`. Returns the post-state with all
/// derived facts integrated into their per-FT cells. Mirrors what
/// `induce::candidate_derives` does after pushing the candidate; we
/// run it here BEFORE inducing so any user-authored derivation rules
/// in the Sherlock readings get their chance to populate cells the
/// alethic constraints expect (e.g. close-world-assumption negations
/// over Evidence-Source-Reliability rows).
fn forward_chain_state(d: &Object) -> Object {
    let refs_owned: Vec<(String, Func)> = ast::cells_iter(d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:") || n.starts_with("derivation_strat2:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, d)))
        .collect();
    let refs: Vec<(&str, &Func)> = refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (post, _) = arest::evaluate::forward_chain_defs_state(&refs, d);
    post
}

/// End-to-end #853 acceptance.
///
/// 1. Compile the merged Sherlock schema + test fixture into defs.
/// 2. Forward-chain to LFP so derivation rules populate every cell
///    the alethic gate inspects.
/// 3. Invoke `Func::Platform("induce")` over `Hypothesis_has_Plausibility`
///    with empty `to_explain` (open-ended search). The Plausibility
///    role is value-typed so the substrate's literal-pin Scoring Rule
///    pattern (`* HC has Confidence Score 'N' iff HC has hidden
///    <ValueRole> 'X'`, see
///    `induce::tests::run_search_ranks_hypothesis_candidates_by_confidence_score_descending`)
///    can match against it.
/// 4. Assert:
///    - the engine returns at least one Hypothesis Candidate;
///    - the result is sorted Confidence-Score-descending — the top
///      Hypothesis Candidate's hidden Plausibility is `'plausible'`
///      and the candidate's Hypothesis is `'h1-evidence-supported'`;
///    - the top Hypothesis Candidate carries a non-empty
///      `confidenceScore` binding (proves the Scoring Rule layer ran,
///      not just enumeration);
///    - the bottom-ranked candidate's score is strictly less than the
///      top's score (the Scoring Rules differentiate).
#[test]
fn test_locked_room_top_hypothesis_is_evidence_supported() {
    let state = merged_state();
    let mut defs = compile_to_defs_state(&state);
    defs.push(("induce".to_string(), Func::Platform("induce".to_string())));
    let d_initial = defs_to_state(&defs, &state);
    let d = forward_chain_state(&d_initial);

    // Open-ended search over `Hypothesis_has_Plausibility`. With empty
    // to_explain the search emits every constraint-satisfying candidate
    // (Hypothesis × Plausibility cartesian); Scoring Rules in
    // `reasoning.md` rank them by the Plausibility role value.
    let args = Object::seq(vec![
        Object::seq(vec![
            Object::atom("ft_id"),
            Object::atom("Hypothesis_has_Plausibility"),
        ]),
        Object::seq(vec![
            Object::atom("to_explain"),
            Object::phi(),
        ]),
    ]);
    let result = ast::apply(
        &Func::Def("induce".to_string()),
        &args,
        &d,
    );
    let hyps = result.as_seq().unwrap_or_else(|| panic!(
        "platform_induce must return a Seq of Hypothesis Candidate facts; got {:?}",
        result));

    assert!(
        !hyps.is_empty(),
        "expected at least one Hypothesis Candidate from the test fixture; got 0",
    );

    // Helper: extract (Hypothesis id, Plausibility, Confidence Score)
    // from a Hypothesis Candidate Seq. The hidden-Fact link cell is
    // `Hypothesis_Candidate_has_hidden__Fact` regardless of the
    // candidate's actual role shape — that's the engine-stamped OUTPUT
    // cell name (see `induce::build_hypothesis_candidate`); the
    // pointer's role bindings come from the candidate's projected
    // per-FT shape (here `Hypothesis_has_Plausibility`).
    let summarise = |hyp: &Object| -> (String, String, String) {
        let hidden = ast::fetch_or_phi("Hypothesis_Candidate_has_hidden__Fact", hyp);
        let pointer = hidden.as_seq()
            .and_then(|facts| facts.first().cloned())
            .unwrap_or(Object::phi());
        let h = ast::binding(&pointer, "Hypothesis").unwrap_or("").to_string();
        let p = ast::binding(&pointer, "Plausibility").unwrap_or("").to_string();
        let s = ast::binding(hyp, "confidenceScore").unwrap_or("").to_string();
        (h, p, s)
    };
    let summaries: Vec<(String, String, String)> = hyps.iter().map(summarise).collect();
    // Top-ranked: must be the evidence-supported hypothesis paired with
    // its 'plausible' Plausibility binding.
    let (top_h, top_p, top_s) = &summaries[0];
    assert_eq!(top_h, "h1-evidence-supported",
        "top-ranked Hypothesis Candidate should be 'h1-evidence-supported' \
         (the one tagged 'plausible' in the fixture); got '{}' with \
         Plausibility '{}', score '{}'. All candidates: {:?}",
        top_h, top_p, top_s, summaries);
    assert_eq!(top_p, "plausible",
        "top-ranked candidate's hidden Plausibility should be 'plausible'; \
         got '{}'. All candidates: {:?}", top_p, summaries);

    // The Scoring Rule layer must have stamped a non-empty score.
    assert!(!top_s.is_empty(),
        "top-ranked Hypothesis Candidate must carry a non-empty \
         `confidenceScore` binding (proves Scoring Rules fired); got \
         empty. All candidates: {:?}", summaries);

    // Strict ranking: the bottom candidate's score must be < top's
    // score. Both are integer strings — parse and compare.
    let top_score: i64 = top_s.parse().unwrap_or_else(|_| panic!(
        "top score must be an integer string for ranking; got '{}'", top_s));
    let bottom_score: i64 = summaries.last().expect("at least one")
        .2.parse().unwrap_or(0);
    assert!(top_score > bottom_score,
        "expected top score ({}) strictly greater than bottom score \
         ({}); ranking didn't differentiate. All candidates: {:?}",
        top_score, bottom_score, summaries);
}
