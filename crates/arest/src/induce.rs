// crates/arest/src/induce.rs
//
// Induction engine search primitives (#848-#852).
//
// This file resurrects the historical `induce.rs` (deleted in
// 77edd7b3 per #211 because it had zero production callers and
// self-referential tests) with a NEW purpose: search primitives
// for the platform `induce` Func registered in #846.
//
// Today's primitive is `enumerate_candidates_for_fact_type` —
// given a FactType id and the current cell state, enumerate every
// candidate fact of that shape across the finite domain of each
// role. Domains come from cells (EnumValues for value types, the
// existing entity population for entity types), NOT hardcoded —
// per the "readings as source code" discipline, the SEMANTICS of
// "what's a candidate" lives in the FactType + Noun + EnumValues
// cells (data); this Rust function is just the dispatch wiring.
//
// #849 (forward-chain check) and #850 (constraint-violation gate)
// will add sibling helpers here that consume the candidates this
// function emits. #851 ties them together into the search loop
// that produces Hypothesis Candidates.

use alloc::{string::{String, ToString}, vec::Vec};
use crate::ast::{Object, fetch_or_phi, fact_from_pairs, binding};

/// Enumerate every candidate fact of shape `ft_id` over the finite
/// domain of each role.
///
/// For each role of the FactType:
///   - If the role's noun is a value type (Noun cell entry where
///     `objectType == "value"`), the domain is the EnumValues cell
///     entry for that noun (all `valueN` bindings).
///   - If the role's noun is an entity type, the domain is the
///     existing population of that noun: any fact in any cell that
///     binds that noun id (e.g. `(Coin → c1)`) contributes the
///     bound value. Duplicates collapse via first-occurrence
///     dedup so a noun that appears in many cells doesn't multiply.
///
/// The cartesian product across roles produces the candidate list.
/// Each candidate is shaped like `InstanceFact`:
///
///   <<subjectNoun, X>, <subjectValue, ...>, <fieldName, FT_id>,
///    <objectNoun, Y>, <objectValue, ...>>
///
/// for binary FTs (drop the object pair for unary; append
/// `roleNNoun`/`roleNValue` for ternary+ — mirrors
/// `parse_forml2_stage2::translate_instance_facts_with_ft_ids`).
///
/// Returns an empty vec when:
///   - the FactType id is not declared,
///   - any role has an empty domain (cartesian product over the
///     empty set IS the empty set — there's no valid binding).
pub fn enumerate_candidates_for_fact_type(state: &Object, ft_id: &str) -> Vec<Object> {
    let role_nouns = role_nouns_for_ft(state, ft_id);
    if role_nouns.is_empty() {
        return Vec::new();
    }
    // Domain per role. Empty domain at any position collapses the
    // whole cartesian product (consistent with set-theoretic ∏).
    let domains: Vec<Vec<String>> = role_nouns.iter()
        .map(|noun| domain_for_noun(state, noun))
        .collect();
    if domains.iter().any(|d| d.is_empty()) {
        return Vec::new();
    }
    // Cartesian product. Iterative to avoid recursion stack hits on
    // wide FT shapes. Index vector advances right-to-left.
    let mut out: Vec<Object> = Vec::new();
    let mut indices: Vec<usize> = vec![0; domains.len()];
    loop {
        let bindings: Vec<&str> = indices.iter()
            .enumerate()
            .map(|(i, &j)| domains[i][j].as_str())
            .collect();
        out.push(build_candidate_fact(ft_id, &role_nouns, &bindings));
        if !advance_indices(&mut indices, &domains) { break; }
    }
    out
}

/// Read role nouns for a FactType id from the Role cell, ordered by
/// `position`. Returns an empty vec when the FactType id is unknown
/// or carries no roles.
fn role_nouns_for_ft(state: &Object, ft_id: &str) -> Vec<String> {
    // Confirm the FT itself exists; if not, no candidates regardless
    // of what roles happen to be lying around.
    let ft_cell = fetch_or_phi("FactType", state);
    let ft_seq = match ft_cell.as_seq() {
        Some(s) => s,
        None => return Vec::new(),
    };
    if !ft_seq.iter().any(|f| binding(f, "id") == Some(ft_id)) {
        return Vec::new();
    }
    let role_cell = fetch_or_phi("Role", state);
    let role_seq = match role_cell.as_seq() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut with_pos: Vec<(usize, String)> = role_seq.iter()
        .filter_map(|r| {
            if binding(r, "factType") != Some(ft_id) { return None; }
            let pos: usize = binding(r, "position")?.parse().ok()?;
            let noun = binding(r, "nounName")?.to_string();
            Some((pos, noun))
        })
        .collect();
    with_pos.sort_by_key(|(p, _)| *p);
    with_pos.into_iter().map(|(_, n)| n).collect()
}

/// Resolve the finite domain for a noun, dispatching on its
/// `objectType` from the Noun cell.
fn domain_for_noun(state: &Object, noun_name: &str) -> Vec<String> {
    let object_type = noun_object_type(state, noun_name);
    match object_type.as_deref() {
        Some("value") => enum_values_for_noun(state, noun_name),
        // Entity (or unknown — treat as entity per the conservative
        // default in compile.rs:953). Walk cells for facts that bind
        // that noun id.
        _ => entity_population_for_noun(state, noun_name),
    }
}

/// Read `objectType` for a noun from the Noun cell. Returns `None`
/// if the noun is undeclared.
fn noun_object_type(state: &Object, noun_name: &str) -> Option<String> {
    let cell = fetch_or_phi("Noun", state);
    let seq = cell.as_seq()?;
    for f in seq.iter() {
        if binding(f, "name") == Some(noun_name) {
            return binding(f, "objectType").map(String::from);
        }
    }
    None
}

/// Read enum values for a value-typed noun from the EnumValues
/// cell. Returns an empty vec when no row matches.
fn enum_values_for_noun(state: &Object, noun_name: &str) -> Vec<String> {
    let cell = fetch_or_phi("EnumValues", state);
    let seq = match cell.as_seq() {
        Some(s) => s,
        None => return Vec::new(),
    };
    for f in seq.iter() {
        if binding(f, "noun") != Some(noun_name) { continue; }
        return (0..)
            .map_while(|i| {
                let key = alloc::format!("value{i}");
                binding(f, &key).map(String::from)
            })
            .collect();
    }
    Vec::new()
}

/// Walk every cell in `state` and collect distinct values bound to
/// the noun id. Mirrors `compile::instances_of_noun_func`'s shape:
/// any binding whose key equals `noun_name` contributes its value.
/// Also reads InstanceFact's `subjectNoun`/`objectNoun` keying so
/// raw stage-2 output (before per-FT cell projection) participates.
fn entity_population_for_noun(state: &Object, noun_name: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut push_unique = |v: &str, seen: &mut Vec<String>| {
        if !v.is_empty() && !seen.iter().any(|s| s == v) {
            seen.push(v.to_string());
        }
    };
    // Walk all cells. Each cell is `<CELL, name, contents>`; the
    // `cells_iter` helper isn't exported, so we walk the Map / Seq
    // representation directly here. Either Object::Map or Object::Seq
    // of cell triples is supported (Backus §13.3.4 + AREST `store`).
    let cells: Vec<(String, Object)> = match state {
        Object::Map(m) => m.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        Object::Seq(items) => items.iter()
            .filter_map(|c| {
                let parts = c.as_seq()?;
                if parts.len() == 3 && parts[0].as_atom() == Some("CELL") {
                    Some((parts[1].as_atom()?.to_string(), parts[2].clone()))
                } else {
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    for (_cell_name, contents) in cells.iter() {
        let Some(facts) = contents.as_seq() else { continue };
        for fact in facts.iter() {
            // Per-FT projection shape: binding key IS the noun name.
            if let Some(v) = binding(fact, noun_name) {
                push_unique(v, &mut seen);
            }
            // Raw InstanceFact shape: subject/object pairs keyed by
            // `subjectNoun`/`subjectValue` (and roleN counterparts).
            if binding(fact, "subjectNoun") == Some(noun_name) {
                if let Some(v) = binding(fact, "subjectValue") {
                    push_unique(v, &mut seen);
                }
            }
            if binding(fact, "objectNoun") == Some(noun_name) {
                if let Some(v) = binding(fact, "objectValue") {
                    push_unique(v, &mut seen);
                }
            }
            // Ternary+ role positions.
            for n in 2.. {
                let noun_key = alloc::format!("role{}Noun", n);
                let Some(other) = binding(fact, &noun_key) else { break };
                if other == noun_name {
                    let val_key = alloc::format!("role{}Value", n);
                    if let Some(v) = binding(fact, &val_key) {
                        push_unique(v, &mut seen);
                    }
                }
            }
        }
    }
    seen
}

/// Advance the cartesian-product index vector. Returns false when
/// the rightmost rollover would carry past position 0 (i.e., the
/// product has been fully enumerated).
fn advance_indices(indices: &mut [usize], domains: &[Vec<String>]) -> bool {
    let mut i = indices.len();
    while i > 0 {
        i -= 1;
        if indices[i] + 1 < domains[i].len() {
            indices[i] += 1;
            for j in (i + 1)..indices.len() {
                indices[j] = 0;
            }
            return true;
        }
    }
    false
}

/// Build a single InstanceFact-shaped candidate. Mirrors
/// `parse_forml2_stage2::translate_instance_facts_with_ft_ids`'s
/// canonical layout: 5-pair prefix (subject + field + object) plus
/// one (roleNNoun, roleNValue) pair per additional role. Unary FTs
/// emit empty objectNoun/objectValue (same as the stage-2 fallback).
fn build_candidate_fact(
    ft_id: &str,
    role_nouns: &[String],
    role_values: &[&str],
) -> Object {
    let subject_noun = role_nouns[0].as_str();
    let subject_value = role_values[0];
    let (object_noun, object_value) = role_nouns.get(1)
        .map(|n| (n.as_str(), role_values[1]))
        .unwrap_or(("", ""));
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(
        5 + 2 * role_nouns.len().saturating_sub(2));
    pairs.push(("subjectNoun".to_string(),  subject_noun.to_string()));
    pairs.push(("subjectValue".to_string(), subject_value.to_string()));
    pairs.push(("fieldName".to_string(),    ft_id.to_string()));
    pairs.push(("objectNoun".to_string(),   object_noun.to_string()));
    pairs.push(("objectValue".to_string(),  object_value.to_string()));
    for (i, noun) in role_nouns.iter().enumerate().skip(2) {
        pairs.push((alloc::format!("role{}Noun", i),  noun.clone()));
        pairs.push((alloc::format!("role{}Value", i), role_values[i].to_string()));
    }
    let pair_refs: Vec<(&str, &str)> = pairs.iter()
        .map(|(k, v)| (k.as_str(), v.as_str())).collect();
    fact_from_pairs(&pair_refs)
}

/// #850 — Per-candidate constraint-violation gate. Build state' =
/// observations + candidate, run the existing `validate` def, return
/// true iff no violations surface.
///
/// Reuses `ast::apply(Func::Def("validate"), ctx, d)` and
/// `ast::decode_violations` — same shape as the compile-rejection
/// path for alethic constraints (see
/// `mc_violation_alethic_rejects_at_compile_time` in
/// `crates/arest/src/compile.rs`).
///
/// `candidate` is shaped as an InstanceFact (canonical layout from
/// `enumerate_candidates_for_fact_type` / stage-2): bindings carry
/// `subjectNoun` / `subjectValue` / `fieldName` / `objectNoun` /
/// `objectValue` (+ `roleNNoun` / `roleNValue` for ternary+). The
/// `fieldName` binding names the FactType id whose cell receives the
/// candidate when projected into per-cell shape — same projection
/// `parse_forml2_stage2::instance_fact_field_cells` performs at
/// stage-2 build time.
///
/// `defs` is the already-compiled defs Object (built by the caller
/// once via `compile::compile_to_defs_state` + `ast::defs_to_state`)
/// so the search loop in #851 can reuse one compile across many
/// candidate checks.
///
/// Returns `true` when no `validate` violation surfaces (candidate is
/// admissible) and `false` otherwise. A candidate with no `fieldName`
/// (or an empty one) is treated as inert — there's no cell to push it
/// into, so it cannot violate anything; returns `true`.
pub fn candidate_passes_constraints(
    state: &Object,
    defs: &Object,
    candidate: &Object,
) -> bool {
    let Some(ft_id) = binding(candidate, "fieldName") else { return true; };
    if ft_id.is_empty() { return true; }
    // Project the InstanceFact-shaped candidate to the per-FT cell
    // shape `validate` reads. Reuses #849's projection helper so the
    // gate and the chain-check stay shape-aligned (one source of
    // truth for cell-fact translation).
    let projected = project_instance_fact_to_per_ft(candidate);
    let state_prime = crate::ast::cell_push(ft_id, projected, state);
    // Encode eval context off the candidate-augmented state so
    // `validate`'s constraint funcs (which read the population via
    // Selector(4) / `extract_facts_func`) see the candidate alongside
    // the existing observations. `defs` carries the compiled Func
    // table — including `validate` itself — built once by the caller.
    let ctx = crate::ast::encode_eval_context_state("", None, &state_prime);
    let violations_obj = crate::ast::apply(
        &crate::ast::Func::Def("validate".to_string()),
        &ctx,
        defs,
    );
    crate::ast::decode_violations(&violations_obj).is_empty()
}

/// #849 — Per-candidate forward-chain check. Build state' = observations
/// + candidate, run forward_chain_defs_state, return whether every fact
/// in `to_explain` is present in the LFP closure.
///
/// Reuses `ast::forward_chain_defs_state` and `ast::cells_iter` — no
/// new combining-form wiring. The candidate is shaped as InstanceFact
/// (canonical 5-pair-prefix layout from #848); the function projects it
/// into the appropriate per-FT cell (per `fieldName` binding) before
/// chain. Mirrors the projection
/// `parse_forml2_stage2::instance_fact_field_cells` performs at stage-2
/// build time so the derivation rule sees the same shape it would in a
/// regular load path. `to_explain` facts use the same InstanceFact
/// shape and are projected the same way for membership lookup.
///
/// `state` carries the observation cells (per-FT cells the candidate
/// pushes into and to_explain reads from). `defs` is the already-
/// compiled defs Object (built by the caller once via
/// `compile::compile_to_defs_state` + `ast::defs_to_state`) so the
/// search loop in #851 can reuse one compile across many candidate
/// checks. We merge state' onto defs so the chained input carries
/// both the rule cells and the post-candidate observation cells.
///
/// Returns `false` early when:
///   - the candidate has no `fieldName` binding (or it's empty) — no
///     cell to push to means no derivation can fire from it;
///   - any fact in `to_explain` lacks a `fieldName` (we cannot locate
///     the cell to look it up in).
/// Returns `true` only when EVERY `to_explain` fact materializes in
/// post-state cells after forward-chain LFP.
pub fn candidate_derives(
    state: &Object,
    defs: &Object,
    candidate: &Object,
    to_explain: &[Object],
) -> bool {
    let Some(ft_id) = binding(candidate, "fieldName") else { return false; };
    if ft_id.is_empty() { return false; }

    // Project the InstanceFact-shaped candidate to the per-FT cell
    // shape, then push into state. Same projection
    // `parse_forml2_stage2::instance_fact_field_cells` performs.
    let projected = project_instance_fact_to_per_ft(candidate);
    let state_with_candidate = crate::ast::cell_push(ft_id, projected, state);

    // Merge state' onto defs so the chained input carries both the
    // metadata + rule cells (from defs) AND the post-candidate
    // observation cells. `merge_states` concat-dedups overlapping
    // cells, so existing observation cells in defs (e.g. seeded
    // domain instances) survive alongside the freshly pushed
    // candidate cell.
    let chained_input = crate::ast::merge_states(defs, &state_with_candidate);

    // Collect derivation refs from defs. Cover both stratum-1
    // (positive) and stratum-2 (negation-guarded) prefixes — same
    // dispatch shape `command::create_via_defs` uses, so the
    // candidate-check sees the same rule surface a regular create
    // would.
    let refs_owned: Vec<(String, crate::ast::Func)> = crate::ast::cells_iter(defs).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:") || n.starts_with("derivation_strat2:"))
        .map(|(n, contents)| (n.to_string(), crate::ast::metacompose(contents, defs)))
        .collect();
    let refs: Vec<(&str, &crate::ast::Func)> = refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();

    // Forward-chain to LFP. The returned post_state has all derived
    // facts integrated into their respective per-FT cells.
    let (post_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&refs, &chained_input);

    // Verify each to_explain fact materialized in post_state. All-must-
    // hold semantics: a single missing fact ⇒ candidate insufficient.
    to_explain.iter().all(|target| target_in_post_state(target, &post_state))
}

/// #851 — Search loop. Enumerate every candidate of shape `ft_id`,
/// gate by alethic constraints (#850), then by forward-chain coverage
/// of `to_explain` (#849), and emit one Hypothesis Candidate fact per
/// surviving candidate per the schema in `readings/core/induction.md`.
///
/// Empty `to_explain` skips the chain check (open-ended search — emit
/// every constraint-satisfying candidate). Non-empty requires every
/// fact in `to_explain` to materialise after forward-chain LFP on the
/// state augmented with the candidate.
///
/// Each emitted Hypothesis Candidate fact carries:
///   - `id`: deterministic, hyp-{ft_id}-{idx} so callers can de-dup
///     across re-runs.
///   - cell `Hypothesis_Candidate_has_hidden__Fact`: the candidate's
///     projected per-FT shape (the atomic fact whose addition to P
///     would explain the targets).
///   - cell `Hypothesis_Candidate_explains_Fact`: per-target pointers
///     in the same projected per-FT shape (empty when to_explain is
///     empty).
/// No scoring (Confidence Score) is attached — that's #852.
///
/// `state` (used for enumeration + projection) and `defs` (compiled
/// def overlays for `validate` + `derivation:*`) are the same shape
/// the caller already builds for #849/#850. Callers that fetch
/// observations + compiled defs as a single Map (typical: post-
/// `defs_to_state` overlay) can pass it as both arguments.
///
/// Returns an empty Vec when no candidate survives (cartesian product
/// is empty, every candidate violates a constraint, or every candidate
/// fails the chain check). Callers wrap in `Object::seq` for FFP-shape
/// emission.
pub fn run_search(
    state: &Object,
    defs: &Object,
    ft_id: &str,
    to_explain: &[Object],
) -> Vec<Object> {
    let candidates = enumerate_candidates_for_fact_type(state, ft_id);
    let mut scored: Vec<(i64, Object)> = Vec::with_capacity(candidates.len());
    for (idx, candidate) in candidates.iter().enumerate() {
        if !candidate_passes_constraints(state, defs, candidate) { continue; }
        if !to_explain.is_empty()
            && !candidate_derives(state, defs, candidate, to_explain) {
            continue;
        }
        // #852 — Score the surviving candidate by forward-chaining the
        // user-authored Scoring Rules over the candidate-augmented state
        // and reading the Confidence Score consequents the rules emit.
        // The id stamped on the Hypothesis Candidate matches the id used
        // for materialising the Hypothesis_Candidate entity row, so the
        // rule-emitted `Hypothesis Candidate has Confidence Score` rows
        // can be filtered down to the candidate at hand.
        let id = alloc::format!("hyp-{}-{}", ft_id, idx);
        let score = score_candidate(state, defs, candidate, &id);
        let hyp = build_hypothesis_candidate(ft_id, idx, candidate, to_explain, &score);
        scored.push((parse_score(&score), hyp));
    }
    // #852 — Stable sort by Confidence Score descending. Stable so ties
    // preserve enumeration order (deterministic across re-runs given the
    // deterministic id stamping in build_hypothesis_candidate).
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, h)| h).collect()
}

/// #852 — Per-candidate scoring pass. Materialise the Hypothesis Candidate
/// as a `Hypothesis_Candidate` entity-cell row keyed by `id`, push the
/// candidate fact into its FT cell, forward-chain over the resulting
/// state, and read the `Hypothesis_Candidate_has_Confidence_Score` cell
/// rows the user-authored Scoring Rules emit. Return the SUM of all
/// scores keyed to `id` (numeric strings parsed as i64; non-numeric
/// scores fall back to 1 per row, so categorical rules like "trustworthy"
/// still rank higher than no-rule-fired). Empty result means no Scoring
/// Rule fired — score zero.
///
/// Rules that emit a Confidence Score for a different Hypothesis
/// Candidate id than the one being scored (which can happen when a rule
/// universally quantifies over Hypothesis Candidate, picking up older
/// hyps still resident in the cell) are filtered out by the id check.
/// In single-pass scoring (each candidate scored in isolation against a
/// freshly-materialised single-row Hypothesis_Candidate cell) this is
/// belt-and-suspenders.
///
/// Returns the score as a String so build_hypothesis_candidate can stamp
/// it as a `confidenceScore` binding on the hyp without a parse round-
/// trip. Numeric ranking happens in run_search via `parse_score`.
fn score_candidate(
    state: &Object,
    defs: &Object,
    candidate: &Object,
    hyp_id: &str,
) -> String {
    let Some(ft_id) = binding(candidate, "fieldName") else { return String::new(); };
    if ft_id.is_empty() { return String::new(); }

    // Project + push the candidate fact into its FT cell so any Scoring
    // Rule whose antecedent reads that cell (e.g. `Coin has Side
    // 'heads'`) sees the candidate alongside the prior observations.
    let projected = project_instance_fact_to_per_ft(candidate);
    let mut state_prime = crate::ast::cell_push(&ft_id, projected, state);

    // Materialise a single Hypothesis_Candidate entity-cell row keyed by
    // the hyp id so any Scoring Rule whose consequent is `Hypothesis
    // Candidate has Confidence Score …` can bind a Hypothesis Candidate
    // to attach the score to. We isolate the scoring pass to ONE hyp at
    // a time so the resulting Confidence Score rows unambiguously
    // belong to this candidate (no cross-candidate contamination).
    let hyp_row = fact_from_pairs(&[("Hypothesis Candidate", hyp_id)]);
    state_prime = crate::ast::cell_push("Hypothesis_Candidate", hyp_row, &state_prime);

    // Project the candidate's NON-SUBJECT bindings onto a synthetic
    // `Hypothesis_Candidate_has_hidden_<ObjectNoun>` cell row keyed by
    // the hyp id. This is what lets a Scoring Rule like `Hypothesis
    // Candidate has Confidence Score '10' iff Hypothesis Candidate has
    // hidden Side 'heads'.` bind the Hypothesis Candidate AND filter
    // on the candidate's role values in a single antecedent — without
    // a cross-cell join the engine doesn't currently express. The
    // host-side projection materialises one such cell per candidate;
    // the rules in readings consume it.
    //
    // For binary FT `<Subject> has <Object>`: row is `<<Hypothesis
    // Candidate, hyp_id>, <<ObjectNoun>, <ObjectValue>>>` in cell
    // `Hypothesis_Candidate_has_hidden_<ObjectNounUnderscored>`. For
    // unary or higher arities the projection mirrors the candidate's
    // per-FT shape with the subject role replaced by Hypothesis
    // Candidate. Empty objectNoun → unary FT, no synthetic cell.
    if let Some(object_noun) = binding(candidate, "objectNoun") {
        if !object_noun.is_empty() {
            let object_value = binding(candidate, "objectValue").unwrap_or("");
            let cell_name = alloc::format!(
                "Hypothesis_Candidate_has_hidden_{}",
                object_noun.replace(' ', "_"));
            let mut pairs: Vec<(String, String)> = Vec::with_capacity(3);
            pairs.push(("Hypothesis Candidate".to_string(), hyp_id.to_string()));
            pairs.push((object_noun.to_string(), object_value.to_string()));
            // Carry through ternary+ role bindings unchanged so multi-
            // role candidates (e.g. `X gives Y to Z`) surface every
            // role on the synthetic cell.
            for n in 2.. {
                let noun_key = alloc::format!("role{}Noun", n);
                let Some(noun) = binding(candidate, &noun_key) else { break };
                if noun.is_empty() { break; }
                let value_key = alloc::format!("role{}Value", n);
                let value = binding(candidate, &value_key).unwrap_or("");
                pairs.push((noun.to_string(), value.to_string()));
            }
            let pair_refs: Vec<(&str, &str)> = pairs.iter()
                .map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let synth_row = fact_from_pairs(&pair_refs);
            state_prime = crate::ast::cell_push(&cell_name, synth_row, &state_prime);
        }
    }

    // Merge state' onto defs so the chained input carries both rule
    // metadata (defs) AND the augmented observation cells. Same shape
    // candidate_derives uses for its forward-chain pass.
    let chained_input = crate::ast::merge_states(defs, &state_prime);

    // Collect derivation refs from defs — both stratum-1 and stratum-2.
    // User-authored Scoring Rules compile to `derivation:rule_<hash>`
    // defs (per the standard derivation pipeline); they're read off the
    // same prefix candidate_derives uses.
    let refs_owned: Vec<(String, crate::ast::Func)> = crate::ast::cells_iter(defs).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:") || n.starts_with("derivation_strat2:"))
        .map(|(n, contents)| (n.to_string(), crate::ast::metacompose(contents, defs)))
        .collect();
    let refs: Vec<(&str, &crate::ast::Func)> = refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();

    // Forward-chain to LFP. The `Hypothesis_Candidate_has_Confidence_Score`
    // cell of the post_state will carry one row per Scoring Rule that
    // fired for this candidate.
    let (post_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&refs, &chained_input);

    let cs_cell = crate::ast::fetch_or_phi(
        "Hypothesis_Candidate_has_Confidence_Score", &post_state);
    let Some(rows) = cs_cell.as_seq() else { return String::new(); };

    // Sum scores keyed to hyp_id. Numeric strings parse as i64 (sum
    // accumulates additive contributions); non-numeric rows count as
    // one point each so a rule emitting a categorical label still
    // contributes to ranking.
    let mut total: i64 = 0;
    let mut any_row_for_hyp = false;
    for row in rows.iter() {
        let row_hyp = binding(row, "Hypothesis Candidate").unwrap_or("");
        // When the row carries no Hypothesis Candidate binding (rule
        // emits a free-floating Confidence Score), treat it as belonging
        // to this candidate — single-row Hypothesis_Candidate cell means
        // there's no other hyp it could attach to.
        if !row_hyp.is_empty() && row_hyp != hyp_id { continue; }
        any_row_for_hyp = true;
        let cs = binding(row, "Confidence Score").unwrap_or("");
        match cs.parse::<i64>() {
            Ok(n) => { total += n; }
            Err(_) => { total += 1; }
        }
    }
    if !any_row_for_hyp { return String::new(); }
    alloc::format!("{}", total)
}

/// Parse a Confidence Score string to an i64 for ranking. Empty or
/// non-numeric strings sort as 0 (no scoring rule fired or rule emitted
/// a categorical label that didn't carry a numeric weight).
fn parse_score(s: &str) -> i64 {
    s.parse::<i64>().unwrap_or(0)
}

/// Encode a single Hypothesis Candidate per `readings/core/induction.md`.
/// Cell-shaped: a Map carrying `id` + the two link-cells the schema
/// declares (`Hypothesis Candidate has hidden- Fact`,
/// `Hypothesis Candidate explains Fact`). Cell names use the
/// `parse_forml2_stage2` convention `<Subject>_<verb>_<Object>` with
/// hyphenated qualifiers underscore-doubled (matches the FT id
/// `Hypothesis_Candidate_has_hidden__Fact` the FORML2 stage-2 emits
/// from the schema's `has hidden- Fact` reading).
///
/// The `id` binding is deterministic on (`ft_id`, `idx`) so re-running
/// the same search produces stable ids — useful for de-dup across
/// successive #852 scoring passes.
///
/// hidden-Fact carries the candidate's per-FT projected shape (one
/// fact). explains-Fact carries one per-FT projected fact per
/// `to_explain` entry (zero when to_explain is empty).
fn build_hypothesis_candidate(
    ft_id: &str,
    idx: usize,
    candidate: &Object,
    to_explain: &[Object],
    confidence_score: &str,
) -> Object {
    let id = alloc::format!("hyp-{}-{}", ft_id, idx);
    let hidden = Object::Seq(
        Vec::from([project_instance_fact_to_per_ft(candidate)]).into());
    let explains = Object::Seq(
        to_explain.iter()
            .map(project_instance_fact_to_per_ft)
            .collect::<Vec<Object>>()
            .into());
    let mut state = Object::phi();
    state = crate::ast::store(
        "Hypothesis_Candidate_has_hidden__Fact", hidden, &state);
    state = crate::ast::store(
        "Hypothesis_Candidate_explains_Fact", explains, &state);
    // Stamp the id as a top-level binding alongside the link cells so
    // callers can read it via `binding(hyp, "id")` without descending
    // into a cell. Mirrors how FactType cells carry `id` directly.
    let id_pair = Object::seq(vec![
        Object::atom("id"),
        Object::atom(&id),
    ]);
    // #852 — Stamp the rule-emitted Confidence Score (if any) as a
    // `confidenceScore` binding. Empty when no Scoring Rule fired
    // for this candidate (search ran without scoring vocabulary, or
    // every rule's antecedent missed). Callers that want the numeric
    // value parse it back from the binding; ranking happens upstream
    // in run_search via parse_score.
    let cs_pair = Object::seq(vec![
        Object::atom("confidenceScore"),
        Object::atom(confidence_score),
    ]);
    // The result is a Seq carrying the id + score binding pairs
    // followed by the two cell triples. `binding` reads the pairs;
    // `fetch_or_phi` reads the cells. Both shapes coexist because
    // `binding` skips any non-2-Seq element, and `fetch_or_phi` only
    // matches CELL triples.
    let cells = state.as_seq().map(|s| s.to_vec()).unwrap_or_default();
    let mut items: Vec<Object> = Vec::with_capacity(2 + cells.len());
    items.push(id_pair);
    items.push(cs_pair);
    items.extend(cells);
    Object::Seq(items.into())
}

/// Project an InstanceFact-shaped fact into the per-FT cell shape
/// `parse_forml2_stage2::instance_fact_field_cells` produces:
///   - role 0:  `<subjectNoun, subjectValue>`
///   - role 1:  `<objectNoun (or fieldName when objectNoun is empty),
///               objectValue>`
///   - role N≥2: `<roleNNoun, roleNValue>`
/// Empty trailing roles end the chain.
fn project_instance_fact_to_per_ft(candidate: &Object) -> Object {
    let subject_noun = binding(candidate, "subjectNoun").unwrap_or("");
    let subject_value = binding(candidate, "subjectValue").unwrap_or("");
    let object_noun = binding(candidate, "objectNoun").unwrap_or("");
    let object_value = binding(candidate, "objectValue").unwrap_or("");
    let field_name = binding(candidate, "fieldName").unwrap_or("");
    // Mirrors `instance_fact_field_cells`: when the FT is unary
    // (objectNoun empty), the second pair keys on the field_name
    // itself so the per-cell shape stays self-describing.
    let object_key = if object_noun.is_empty() { field_name } else { object_noun };
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(2);
    pairs.push((subject_noun.to_string(), subject_value.to_string()));
    pairs.push((object_key.to_string(),    object_value.to_string()));
    let mut n: usize = 2;
    loop {
        let noun_key = alloc::format!("role{}Noun", n);
        let value_key = alloc::format!("role{}Value", n);
        let Some(noun) = binding(candidate, &noun_key) else { break };
        if noun.is_empty() { break; }
        let value = binding(candidate, &value_key).unwrap_or("");
        pairs.push((noun.to_string(), value.to_string()));
        n += 1;
    }
    let pair_refs: Vec<(&str, &str)> = pairs.iter()
        .map(|(k, v)| (k.as_str(), v.as_str())).collect();
    fact_from_pairs(&pair_refs)
}

/// Membership test for a `to_explain` fact (InstanceFact-shape) in a
/// post-LFP state. Project the target to per-FT shape, fetch the
/// matching per-FT cell, and test for any fact whose bindings
/// superset-cover the target's. Order-independent (per-FT cells write
/// pairs in declared role order, but the check tolerates re-ordering
/// for robustness against future cell-shape evolution).
fn target_in_post_state(target: &Object, post_state: &Object) -> bool {
    let Some(ft_id) = binding(target, "fieldName") else { return false; };
    if ft_id.is_empty() { return false; }
    let projected = project_instance_fact_to_per_ft(target);
    let target_pairs: Vec<(&str, &str)> = match projected.as_seq() {
        Some(items) => items.iter().filter_map(|p| {
            let kv = p.as_seq()?;
            if kv.len() != 2 { return None; }
            Some((kv[0].as_atom()?, kv[1].as_atom()?))
        }).collect(),
        None => return false,
    };
    let cell = fetch_or_phi(ft_id, post_state);
    let Some(facts) = cell.as_seq() else { return false; };
    facts.iter().any(|fact| {
        target_pairs.iter().all(|(k, v)| binding(fact, k) == Some(*v))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{cell, Object};

    /// Build a synthetic state carrying Noun + FactType + Role +
    /// EnumValues + (optional) InstanceFact cells. Mirrors the
    /// shape stage-2 emits so the function exercises real cell
    /// reads, not handcrafted accessors.
    fn make_state(
        nouns: &[(&str, &str)],            // (name, objectType)
        fact_types: &[(&str, &str, usize)], // (id, reading, arity)
        roles: &[(&str, &str, usize)],     // (factType, nounName, position)
        enum_values: &[(&str, &[&str])],   // (noun, values)
        instance_facts: &[Vec<(&str, &str)>],
    ) -> Object {
        let noun_facts: Vec<Object> = nouns.iter()
            .map(|(n, t)| fact_from_pairs(&[("name", n), ("objectType", t)]))
            .collect();
        let ft_facts: Vec<Object> = fact_types.iter()
            .map(|(id, reading, arity)| {
                let arity_s = arity.to_string();
                fact_from_pairs(&[
                    ("id", *id),
                    ("reading", *reading),
                    ("arity", arity_s.as_str()),
                ])
            })
            .collect();
        let role_facts: Vec<Object> = roles.iter()
            .map(|(ft, n, pos)| {
                let pos_s = pos.to_string();
                fact_from_pairs(&[
                    ("factType", *ft),
                    ("nounName", *n),
                    ("position", pos_s.as_str()),
                ])
            })
            .collect();
        let enum_facts: Vec<Object> = enum_values.iter()
            .map(|(noun, vals)| {
                let mut pairs: Vec<(String, String)> = Vec::new();
                pairs.push(("noun".to_string(), (*noun).to_string()));
                for (i, v) in vals.iter().enumerate() {
                    pairs.push((alloc::format!("value{i}"), (*v).to_string()));
                }
                let pair_refs: Vec<(&str, &str)> = pairs.iter()
                    .map(|(k, v)| (k.as_str(), v.as_str())).collect();
                fact_from_pairs(&pair_refs)
            })
            .collect();
        let inst_facts: Vec<Object> = instance_facts.iter()
            .map(|pairs| {
                let pair_refs: Vec<(&str, &str)> = pairs.iter()
                    .map(|(k, v)| (*k, *v)).collect();
                fact_from_pairs(&pair_refs)
            })
            .collect();
        Object::seq(vec![
            cell("Noun",         Object::Seq(noun_facts.into())),
            cell("FactType",     Object::Seq(ft_facts.into())),
            cell("Role",         Object::Seq(role_facts.into())),
            cell("EnumValues",   Object::Seq(enum_facts.into())),
            cell("InstanceFact", Object::Seq(inst_facts.into())),
        ])
    }

    /// Single role over a value type: enumeration matches the
    /// EnumValues cell row 1:1. Drives the value-type domain branch.
    /// For the `Coin has Side` binary FT, with one Coin instance
    /// `c1` and Side ∈ {heads, tails}, we expect 2 candidates:
    /// `<c1, heads>` and `<c1, tails>`.
    #[test]
    fn single_role_enum_value_type_yields_one_per_value() {
        let state = make_state(
            &[("Coin", "entity"), ("Side", "value")],
            &[("Coin_has_Side", "Coin has Side", 2)],
            &[
                ("Coin_has_Side", "Coin", 0),
                ("Coin_has_Side", "Side", 1),
            ],
            &[("Side", &["heads", "tails"])],
            // Single Coin instance c1 — supplies the entity-side
            // domain for role 0 via the InstanceFact cell.
            &[vec![
                ("subjectNoun",  "Coin"),
                ("subjectValue", "c1"),
                ("fieldName",    "Coin_exists"),
                ("objectNoun",   ""),
                ("objectValue",  ""),
            ]],
        );
        let candidates = enumerate_candidates_for_fact_type(&state, "Coin_has_Side");
        assert_eq!(candidates.len(), 2,
            "expected one candidate per Side value (heads, tails); got {:?}",
            candidates);
        let pairs: Vec<(Option<&str>, Option<&str>, Option<&str>, Option<&str>)> =
            candidates.iter().map(|c| (
                binding(c, "subjectNoun"),
                binding(c, "subjectValue"),
                binding(c, "objectNoun"),
                binding(c, "objectValue"),
            )).collect();
        assert!(pairs.contains(&(Some("Coin"), Some("c1"), Some("Side"), Some("heads"))),
            "missing <c1, heads>; got {:?}", pairs);
        assert!(pairs.contains(&(Some("Coin"), Some("c1"), Some("Side"), Some("tails"))),
            "missing <c1, tails>; got {:?}", pairs);
        // fieldName must be the canonical FT id (not a raw verb)
        // since this primitive operates on declared FactType ids.
        for c in candidates.iter() {
            assert_eq!(binding(c, "fieldName"), Some("Coin_has_Side"));
        }
    }

    /// Cartesian product across two roles: 2 entities × 2 enum
    /// values = 4 candidates. Confirms the index advancer.
    #[test]
    fn two_role_binary_ft_yields_cartesian_product() {
        let state = make_state(
            &[("Coin", "entity"), ("Side", "value")],
            &[("Coin_has_Side", "Coin has Side", 2)],
            &[
                ("Coin_has_Side", "Coin", 0),
                ("Coin_has_Side", "Side", 1),
            ],
            &[("Side", &["heads", "tails"])],
            &[
                vec![
                    ("subjectNoun",  "Coin"),
                    ("subjectValue", "c1"),
                    ("fieldName",    "Coin_exists"),
                    ("objectNoun",   ""),
                    ("objectValue",  ""),
                ],
                vec![
                    ("subjectNoun",  "Coin"),
                    ("subjectValue", "c2"),
                    ("fieldName",    "Coin_exists"),
                    ("objectNoun",   ""),
                    ("objectValue",  ""),
                ],
            ],
        );
        let candidates = enumerate_candidates_for_fact_type(&state, "Coin_has_Side");
        assert_eq!(candidates.len(), 4,
            "expected 2 Coins × 2 Sides = 4 candidates; got {:?}",
            candidates);
        // All four (Coin, Side) pairs must be present exactly once.
        let pairs: Vec<(Option<&str>, Option<&str>)> =
            candidates.iter().map(|c| (
                binding(c, "subjectValue"),
                binding(c, "objectValue"),
            )).collect();
        for coin in &["c1", "c2"] {
            for side in &["heads", "tails"] {
                assert!(pairs.contains(&(Some(*coin), Some(*side))),
                    "missing <{}, {}>; got {:?}", coin, side, pairs);
            }
        }
    }

    /// Empty domain at any role collapses the cartesian product.
    /// Side has no enum values declared and no entity instances —
    /// so `Coin has Side` yields zero candidates regardless of how
    /// many Coins there are.
    #[test]
    fn empty_domain_yields_empty_seq() {
        let state = make_state(
            &[("Coin", "entity"), ("Side", "value")],
            &[("Coin_has_Side", "Coin has Side", 2)],
            &[
                ("Coin_has_Side", "Coin", 0),
                ("Coin_has_Side", "Side", 1),
            ],
            // No EnumValues row for Side → role 1's domain is empty.
            &[],
            // Coin still has an instance — role 0's domain is non-empty.
            &[vec![
                ("subjectNoun",  "Coin"),
                ("subjectValue", "c1"),
                ("fieldName",    "Coin_exists"),
                ("objectNoun",   ""),
                ("objectValue",  ""),
            ]],
        );
        let candidates = enumerate_candidates_for_fact_type(&state, "Coin_has_Side");
        assert!(candidates.is_empty(),
            "expected empty result when role domain is empty; got {:?}",
            candidates);
    }

    // ─── #850 candidate_passes_constraints ────────────────────────

    /// Build a state with a UC alethic constraint on the role-0 of an
    /// `X has Foo` binary FT (`Each X has at most one Foo`). Reuses
    /// `make_state` for the schema cells, then layers the Constraint
    /// cell on top via `parse_forml2::constraint_to_fact_test` so the
    /// constraint encoding stays in sync with the production parser.
    fn state_with_uc_each_x_has_at_most_one_foo() -> Object {
        use crate::types::{ConstraintDef, SpanDef};
        let mut state = make_state(
            &[("X", "entity"), ("Foo", "value")],
            &[("X_has_Foo", "X has Foo", 2)],
            &[
                ("X_has_Foo", "X", 0),
                ("X_has_Foo", "Foo", 1),
            ],
            &[("Foo", &["a", "b"])],
            // Seed one X instance via an InstanceFact so the entity-side
            // domain is non-empty for downstream candidate emission.
            &[vec![
                ("subjectNoun",  "X"),
                ("subjectValue", "x1"),
                ("fieldName",    "X_exists"),
                ("objectNoun",   ""),
                ("objectValue",  ""),
            ]],
        );
        let uc = ConstraintDef {
            id: "uc_each_x_at_most_one_foo".into(),
            kind: "UC".into(),
            modality: "Alethic".into(),
            text: "Each X has at most one Foo".into(),
            spans: vec![SpanDef {
                fact_type_id: "X_has_Foo".into(),
                role_index: 0,
                subset_autofill: None,
            }],
            ..Default::default()
        };
        let constraint_fact = crate::parse_forml2::constraint_to_fact_test(&uc);
        // make_state emits a Seq store; cell_push correctly appends a
        // Constraint cell on top, leaving the schema cells intact.
        state = crate::ast::cell_push("Constraint", constraint_fact, &state);
        state
    }

    /// Build an InstanceFact-shaped candidate for `X has Foo` matching
    /// the canonical layout `enumerate_candidates_for_fact_type` emits.
    fn x_has_foo_candidate(x_value: &str, foo_value: &str) -> Object {
        fact_from_pairs(&[
            ("subjectNoun",  "X"),
            ("subjectValue", x_value),
            ("fieldName",    "X_has_Foo"),
            ("objectNoun",   "Foo"),
            ("objectValue",  foo_value),
        ])
    }

    /// One candidate, no observations: a single `<X has Foo>` fact
    /// satisfies `Each X has at most one Foo` (one is ≤ one). The gate
    /// must accept this candidate. Drives the no-violation branch
    /// straight through `validate` — the same path #851's search loop
    /// will exercise on every viable hypothesis.
    #[test]
    fn candidate_satisfying_all_constraints_returns_true() {
        let state = state_with_uc_each_x_has_at_most_one_foo();
        let defs_vec = crate::compile::compile_to_defs_state(&state);
        let d = crate::ast::defs_to_state(&defs_vec, &state);
        let candidate = x_has_foo_candidate("x1", "a");
        assert!(
            candidate_passes_constraints(&state, &d, &candidate),
            "single <x1, Foo a> candidate satisfies UC `Each X has at most one Foo` \
             (count = 1); gate must return true"
        );
    }

    /// Same schema, but observations already contain `<x1 has Foo a>`.
    /// The candidate `<x1 has Foo b>` adds a second Foo for x1 — UC
    /// fires (count = 2 > 1). Gate must reject. Drives the violation
    /// branch through `validate`'s alethic path, mirroring how the
    /// compile-rejection harness in
    /// `mc_violation_alethic_rejects_at_compile_time` (compile.rs)
    /// surfaces alethic violations to the platform.
    #[test]
    fn candidate_violating_alethic_uc_returns_false() {
        let mut state = state_with_uc_each_x_has_at_most_one_foo();
        // Pre-load the FT cell with the existing `<x1, a>` observation
        // in per-cell shape so `validate` reads it identically to a
        // stage-2-emitted fact (matches `instance_fact_field_cells`).
        state = crate::ast::cell_push(
            "X_has_Foo",
            fact_from_pairs(&[("X", "x1"), ("Foo", "a")]),
            &state,
        );
        let defs_vec = crate::compile::compile_to_defs_state(&state);
        let d = crate::ast::defs_to_state(&defs_vec, &state);
        let candidate = x_has_foo_candidate("x1", "b");
        assert!(
            !candidate_passes_constraints(&state, &d, &candidate),
            "candidate <x1, Foo b> on top of observation <x1, Foo a> violates \
             UC `Each X has at most one Foo` (count = 2); gate must return false"
        );
    }

    // ─── #849 candidate_derives ───────────────────────────────────────

    /// Compile a tiny FORML2 schema carrying a literal-pinned iff
    /// derivation rule (`* Thing has Foo 'fired' iff Thing has Bar
    /// 'present'`). Returns the parsed state plus the unified defs
    /// map (state's cells + `derivation:*` overlays) so each test
    /// can push a candidate cell and call `candidate_derives`
    /// without re-paying compile cost. The literal pinning makes the
    /// consequent unambiguous (Foo 'fired' from any Thing whose Bar
    /// is 'present') so the membership check on `to_explain` lines
    /// up exactly with the derived fact.
    fn schema_thing_foo_iff_thing_bar() -> (Object, Object) {
        let src = r#"# Test
Thing(.Id) is an entity type.
Id is a value type.
Foo is a value type.
Bar is a value type.
Other is a value type.

## Fact Types
Thing has Id.
Thing has Foo.
Thing has Bar.
Thing has Other.

## Derivation Rules
* Thing has Foo 'fired' iff Thing has Bar 'present'.
"#;
        let state = crate::parse_forml2::parse_to_state(src).expect("parse");
        let defs_vec = crate::compile::compile_to_defs_state(&state);
        let d = crate::ast::defs_to_state(&defs_vec, &state);
        (state, d)
    }

    /// Build an InstanceFact-shaped fact for `Thing has <Field>
    /// '<value>'` matching the canonical 5-pair layout
    /// `enumerate_candidates_for_fact_type` emits.
    fn thing_has_field_fact(thing_id: &str, field: &str, value: &str) -> Object {
        let ft_id = alloc::format!("Thing_has_{field}");
        fact_from_pairs(&[
            ("subjectNoun",  "Thing"),
            ("subjectValue", thing_id),
            ("fieldName",    ft_id.as_str()),
            ("objectNoun",   field),
            ("objectValue",  value),
        ])
    }

    /// Candidate `<t1 has Bar 'present'>` is exactly the antecedent
    /// of the iff rule. Forward-chain on observations + candidate
    /// must materialise `<t1 has Foo 'fired'>` into the
    /// `Thing_has_Foo` cell. The check then holds — return `true`.
    /// Drives the positive branch through the derivation projection +
    /// LFP closure path #851's search loop will use to confirm a
    /// candidate explains its target.
    #[test]
    fn candidate_triggering_derivation_rule_passes_check() {
        let (state, d) = schema_thing_foo_iff_thing_bar();
        let candidate = thing_has_field_fact("t1", "Bar", "present");
        let to_explain = vec![thing_has_field_fact("t1", "Foo", "fired")];
        assert!(
            candidate_derives(&state, &d, &candidate, &to_explain),
            "candidate <t1 has Bar 'present'> must trigger derivation \
             `Thing has Foo 'fired' iff Thing has Bar 'present'` and \
             materialise <t1 has Foo 'fired'>"
        );
    }

    /// Same schema, different candidate: `<t1 has Other 'whatever'>`
    /// touches a different FT cell (Thing_has_Other), so the
    /// antecedent of the rule (Thing_has_Bar with Bar='present')
    /// remains empty after push. No derivation fires; `<t1 has Foo
    /// 'fired'>` does not materialise; the check returns `false`.
    /// Drives the negative branch — the gate that lets #851's search
    /// loop discard candidates whose closure misses the target set.
    #[test]
    fn candidate_insufficient_to_trigger_rule_returns_false() {
        let (state, d) = schema_thing_foo_iff_thing_bar();
        let candidate = thing_has_field_fact("t1", "Other", "whatever");
        let to_explain = vec![thing_has_field_fact("t1", "Foo", "fired")];
        assert!(
            !candidate_derives(&state, &d, &candidate, &to_explain),
            "candidate <t1 has Other 'whatever'> does not match the \
             antecedent `Thing has Bar 'present'`; <t1 has Foo \
             'fired'> must NOT derive"
        );
    }

    // ─── #851 run_search — search loop integration ───────────────────

    /// Acceptance test for the platform_induce search loop (#851).
    ///
    /// Schema:
    ///   Coin(.id) is an entity type. Side is a value type.
    ///   Side enum: 'heads', 'tails'. Coin has Side.
    ///   Each Coin has at most one Side.
    ///
    /// Observations: one Coin instance c1, no Side asserted.
    /// to_explain: empty (open-ended search — emit every constraint-
    /// satisfying candidate).
    ///
    /// The search loop must:
    ///   1. Enumerate `Coin_has_Side` candidates over the c1 × {heads,
    ///      tails} domain → 2 candidates.
    ///   2. Each candidate satisfies the UC (count=1 ≤ 1) — both pass
    ///      `candidate_passes_constraints`.
    ///   3. With empty `to_explain`, the chain-check is skipped — both
    ///      candidates emit Hypothesis Candidates.
    ///   4. Result is a Seq of length 2; each Hypothesis Candidate has
    ///      a hidden-Fact link to its candidate's projected per-FT
    ///      atom; explains-Fact is empty (no to_explain).
    #[test]
    fn coin_side_no_to_explain_yields_one_hypothesis_per_enum_value() {
        use crate::ast::{Func, apply, defs_to_state, fetch_or_phi};
        use crate::types::{ConstraintDef, SpanDef};
        let mut state = make_state(
            &[("Coin", "entity"), ("Side", "value")],
            &[("Coin_has_Side", "Coin has Side", 2)],
            &[
                ("Coin_has_Side", "Coin", 0),
                ("Coin_has_Side", "Side", 1),
            ],
            &[("Side", &["heads", "tails"])],
            // One Coin instance c1, no Side asserted yet.
            &[vec![
                ("subjectNoun",  "Coin"),
                ("subjectValue", "c1"),
                ("fieldName",    "Coin_exists"),
                ("objectNoun",   ""),
                ("objectValue",  ""),
            ]],
        );
        // Layer the UC alethic constraint via the production
        // constraint encoder so the gate exercises the same path as
        // #850's tests.
        let uc = ConstraintDef {
            id: "uc_each_coin_at_most_one_side".into(),
            kind: "UC".into(),
            modality: "Alethic".into(),
            text: "Each Coin has at most one Side".into(),
            spans: vec![SpanDef {
                fact_type_id: "Coin_has_Side".into(),
                role_index: 0,
                subset_autofill: None,
            }],
            ..Default::default()
        };
        let constraint_fact = crate::parse_forml2::constraint_to_fact_test(&uc);
        state = crate::ast::cell_push("Constraint", constraint_fact, &state);

        // Compile defs from state so the search loop has access to the
        // `validate` def + `derivation:*` overlays, then register the
        // induce platform itself so apply() routes through.
        let mut defs_vec = crate::compile::compile_to_defs_state(&state);
        defs_vec.push((
            "induce".to_string(),
            Func::Platform("induce".to_string()),
        ));
        let d = defs_to_state(&defs_vec, &state);

        // Build args: <<ft_id, "Coin_has_Side">, <to_explain, <>>>.
        // Empty to_explain triggers the open-ended branch (no chain-
        // check; emit every constraint-satisfying candidate).
        let args = Object::seq(vec![
            Object::seq(vec![
                Object::atom("ft_id"),
                Object::atom("Coin_has_Side"),
            ]),
            Object::seq(vec![
                Object::atom("to_explain"),
                Object::phi(),
            ]),
        ]);

        let result = apply(&Func::Def("induce".to_string()), &args, &d);

        // Result must be a Seq of 2 Hypothesis Candidate facts.
        let hyps = result.as_seq().expect(
            "platform_induce must return a Seq of Hypothesis Candidate facts");
        assert_eq!(hyps.len(), 2,
            "expected 2 Hypothesis Candidates (one per Side enum value); \
             got {} → {:?}", hyps.len(), hyps);

        // Each Hypothesis Candidate must have an id and a hidden-Fact
        // pointer; with empty to_explain, no explains-Fact pointers.
        // hidden-Facts collectively span the projected per-FT shape of
        // both candidates (`<<Coin, c1>, <Side, heads>>` and
        // `<<Coin, c1>, <Side, tails>>`).
        let mut sides_seen: Vec<String> = Vec::new();
        for hyp in hyps.iter() {
            let hyp_id = binding(hyp, "id").expect(
                "every Hypothesis Candidate must carry an id binding");
            assert!(!hyp_id.is_empty(),
                "Hypothesis Candidate id must not be empty: {:?}", hyp);
            // The candidate's projected per-FT shape lives on the
            // `Coin_has_Side` cell of the post-state attached to the
            // hyp via hidden-Fact. Grab the cell's binding for `Side`
            // and confirm it's one of {heads, tails}.
            let hidden = fetch_or_phi("Hypothesis_Candidate_has_hidden__Fact", hyp);
            let hidden_facts = hidden.as_seq().expect(
                "hidden-Fact link must be present as a Seq of pointers");
            assert_eq!(hidden_facts.len(), 1,
                "single-candidate Hypothesis Candidate must have one \
                 hidden-Fact pointer; got {:?}", hidden_facts);
            let pointer = &hidden_facts[0];
            // Pointer is a fact carrying the candidate's per-FT shape:
            // `<<Coin, c1>, <Side, X>>`.
            assert_eq!(binding(pointer, "Coin"), Some("c1"),
                "hidden-Fact pointer must reference Coin c1; got {:?}",
                pointer);
            let side_value = binding(pointer, "Side").expect(
                "hidden-Fact pointer must carry a Side binding");
            sides_seen.push(side_value.to_string());
            // explains-Fact is empty when to_explain is empty.
            let explains = fetch_or_phi("Hypothesis_Candidate_explains_Fact", hyp);
            let explains_seq = explains.as_seq().expect(
                "explains-Fact link must be present (possibly empty Seq) \
                 even when no to_explain facts are supplied");
            assert!(explains_seq.is_empty(),
                "explains-Fact must be empty when to_explain is empty; \
                 got {:?}", explains_seq);
        }
        sides_seen.sort();
        assert_eq!(sides_seen, vec!["heads".to_string(), "tails".to_string()],
            "expected one Hypothesis Candidate per Side enum value \
             (heads, tails); got {:?}", sides_seen);
    }

    // ─── #852 Scoring Rules — rank Hypothesis Candidates by Confidence Score ──

    /// Acceptance test for the platform_induce scoring + ranking layer (#852).
    ///
    /// "Scoring Rules as readings, not Rust" — the test schema declares two
    /// derivation rules that emit `Hypothesis Candidate has Confidence Score`
    /// facts. The induce loop materialises each surviving Hypothesis
    /// Candidate into the `Hypothesis_Candidate` entity cell, forward-chains
    /// the user-authored rules over the candidate-augmented state, reads the
    /// score off the resulting `Hypothesis_Candidate_has_Confidence_Score`
    /// cell, and sorts hyps by score descending. There is NO hardcoded
    /// scoring function in Rust — the readings ARE the source code.
    ///
    /// Schema:
    ///   Coin(.id) is an entity type. Side is a value type with values
    ///   {'heads','tails'}. Coin has Side. Each Coin has at most one Side.
    ///   Hypothesis Candidate(.id) is an entity type. Confidence Score is
    ///   a value type. Hypothesis Candidate has Confidence Score.
    ///   Hypothesis Candidate has hidden Side.
    ///   * Hypothesis Candidate has Confidence Score '10'
    ///       iff Hypothesis Candidate has hidden Side 'heads'.
    ///   * Hypothesis Candidate has Confidence Score '1'
    ///       iff Hypothesis Candidate has hidden Side 'tails'.
    ///
    /// The `Hypothesis Candidate has hidden Side` FT is the surface the
    /// induce engine projects per-candidate role bindings onto so each
    /// rule's antecedent can BIND a Hypothesis Candidate (from the
    /// projected row) AND filter on the candidate's Side value (literal
    /// pin) in a single antecedent — the consequent reuses the
    /// antecedent's Hypothesis Candidate binding to attach the score.
    ///
    /// Observations: one Coin instance c1, no Side asserted.
    /// to_explain: empty.
    ///
    /// Search loop must:
    ///   1. Enumerate `Coin_has_Side` candidates → 2 candidates (c1×heads, c1×tails).
    ///   2. Both pass the at-most-one-Side UC.
    ///   3. Per-candidate scoring pass:
    ///      - Materialise Hypothesis Candidate entity row for the hyp id.
    ///      - Project the candidate fact onto a per-hyp
    ///        `Hypothesis_Candidate_has_hidden_Side` row.
    ///      - Forward-chain → Rule A fires for heads-candidate (score '10');
    ///        Rule B fires for tails-candidate (score '1').
    ///   4. Result is a Seq ordered by Confidence Score DESCENDING:
    ///      first = heads-hyp (score 10), second = tails-hyp (score 1).
    #[test]
    fn run_search_ranks_hypothesis_candidates_by_confidence_score_descending() {
        use crate::ast::{Func, apply, defs_to_state, fetch_or_phi};
        // Schema — Coin/Side enumeration fixture + scoring vocabulary.
        // Two Scoring Rules expressed as FORML2 derivation rules. The
        // induce engine materialises a per-candidate
        // `Hypothesis_Candidate_has_hidden_Side` row keyed by hyp id
        // alongside the `Hypothesis_Candidate` entity row, then forward-
        // chains the rules over that augmented state. Each rule's
        // antecedent binds Hypothesis Candidate (from
        // `Hypothesis Candidate has hidden Side`) AND filters Side
        // (literal pin); the consequent reuses the antecedent's
        // Hypothesis Candidate binding to attach a Confidence Score
        // pinned to the rule's literal.
        let src = "# Coin/Side scoring fixture\n\
            Coin(.id) is an entity type.\n\
            Side is a value type.\n\
            The possible values of Side are 'heads', 'tails'.\n\
            Coin has Side.\n\
            Each Coin has at most one Side.\n\
            \n\
            Hypothesis Candidate(.id) is an entity type.\n\
            Confidence Score is a value type.\n\
            Hypothesis Candidate has Confidence Score.\n\
            Hypothesis Candidate has hidden Side.\n\
            \n\
            ## Derivation Rules\n\
            * Hypothesis Candidate has Confidence Score '10' iff Hypothesis Candidate has hidden Side 'heads'.\n\
            * Hypothesis Candidate has Confidence Score '1' iff Hypothesis Candidate has hidden Side 'tails'.\n\
        ";
        let state = crate::parse_forml2::parse_to_state(src).expect("parse");
        // Seed one Coin instance c1 via an InstanceFact so the entity-side
        // domain for `Coin_has_Side` enumeration is non-empty. The
        // schema declares Coin as an entity type; the `entity_population_for_noun`
        // helper walks cells for Coin bindings, picking this up.
        let state = crate::ast::cell_push(
            "InstanceFact",
            fact_from_pairs(&[
                ("subjectNoun",  "Coin"),
                ("subjectValue", "c1"),
                ("fieldName",    "Coin_exists"),
                ("objectNoun",   ""),
                ("objectValue",  ""),
            ]),
            &state,
        );
        let mut defs_vec = crate::compile::compile_to_defs_state(&state);
        defs_vec.push((
            "induce".to_string(),
            Func::Platform("induce".to_string()),
        ));
        let d = defs_to_state(&defs_vec, &state);

        let args = Object::seq(vec![
            Object::seq(vec![
                Object::atom("ft_id"),
                Object::atom("Coin_has_Side"),
            ]),
            Object::seq(vec![
                Object::atom("to_explain"),
                Object::phi(),
            ]),
        ]);

        let result = apply(&Func::Def("induce".to_string()), &args, &d);
        let hyps = result.as_seq().expect(
            "platform_induce must return a Seq of Hypothesis Candidate facts");
        assert_eq!(hyps.len(), 2,
            "expected 2 Hypothesis Candidates (one per Side enum value); \
             got {} → {:?}", hyps.len(), hyps);

        // Read each hyp's Confidence Score (top-level binding stamped by
        // build_hypothesis_candidate after #852 wires scoring) and the
        // associated Side from its hidden-Fact pointer.
        let hyp_summaries: Vec<(String, String)> = hyps.iter().map(|hyp| {
            let score = binding(hyp, "confidenceScore")
                .map(String::from)
                .unwrap_or_default();
            let hidden = fetch_or_phi("Hypothesis_Candidate_has_hidden__Fact", hyp);
            let side = hidden.as_seq()
                .and_then(|facts| facts.first().cloned())
                .and_then(|f| binding(&f, "Side").map(String::from))
                .unwrap_or_default();
            (score, side)
        }).collect();

        // Both candidates must carry a non-empty score binding (proves the
        // scoring layer ran, not just enumeration).
        for (score, side) in &hyp_summaries {
            assert!(!score.is_empty(),
                "Hypothesis Candidate (Side={}) must carry a confidenceScore \
                 binding; summaries={:?}", side, hyp_summaries);
        }

        // Ordering: heads (score=10) must precede tails (score=1).
        assert_eq!(hyp_summaries[0].1, "heads",
            "expected first hyp to be Side=heads (score 10); got {:?}",
            hyp_summaries);
        assert_eq!(hyp_summaries[0].0, "10",
            "expected first hyp's confidenceScore to be '10'; got {:?}",
            hyp_summaries);
        assert_eq!(hyp_summaries[1].1, "tails",
            "expected second hyp to be Side=tails (score 1); got {:?}",
            hyp_summaries);
        assert_eq!(hyp_summaries[1].0, "1",
            "expected second hyp's confidenceScore to be '1'; got {:?}",
            hyp_summaries);
    }
}
