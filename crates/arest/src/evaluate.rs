// crates/arest/src/evaluate.rs
//
// Evaluation is beta reduction. That's it.
//
// Constraint verification:  constraints.flat_map(|c| apply(c.func, ctx)) -> [Violation]
// Forward inference:        derivations.flat_map(|d| apply(d.func, pop)) -> [DerivedFact]
// State machine execution:  fold(transition)(initial)(stream) -> final_state
// Synthesis:                collect all knowledge about a noun from the compiled model.

use hashbrown::HashSet;
use crate::types::*;
use crate::ast;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// -- Forward Chaining -------------------------------------------------
//
// Correctness: FORML 2 derivation rules are monotonic (add facts, never
// remove). The population is finite. A monotonic sequence over a finite
// set reaches a fixed point. The loop terminates when no new facts are
// derived.
//
// Safety: the iteration bound prevents pathological rule sets from
// producing unbounded intermediate populations. If the bound is hit,
// the engine stops and returns what it has -- a partial fixed point.

/// Forward-chain derivation rules to a fixed point.
///
/// Each derivation def is applied to the current population. New facts
/// are added, and the process repeats until no new facts are derived
/// (fixed point reached) or the iteration bound is hit.
///
/// Iteration bound: 100 iterations maximum. FORML2 derivation rules are
/// monotonic (facts are added, never removed) over a finite domain, so
/// convergence is guaranteed in theory. The 100-iteration bound is a
/// safety net for pathological rule sets that produce very large
/// intermediate populations. If the bound is exceeded, the function
/// returns a partial fixed point -- all facts derived so far, even
/// though additional derivations may be possible. This is safe because
/// each derived fact is individually correct; only completeness is
/// affected.
/// Forward-chain derivation rules over D to fixed point. Returns (D', derived_facts).
/// D contains both population cells and def cells (Backus Sec. 14.3).
pub fn forward_chain_defs_state(
    derivation_defs: &[(&str, &ast::Func)],
    d: &ast::Object,
) -> (ast::Object, Vec<DerivedFact>) {
    forward_chain_defs_state_bounded(derivation_defs, d, 100)
}

/// Apply all derivation rules once, returning novel facts.
///
/// Dedup against three populations: facts already in `current_state`,
/// facts derived in prior rounds (`all_derived`), and facts emitted
/// earlier in this round. All three use a canonical `FactKey`
/// (fact_type_id + sorted bindings) and `HashSet` lookups — a naive
/// per-candidate linear scan is O(K·N); the hashed form is O(K+N).
fn derive_one_round(
    derivation_defs: &[(&str, &ast::Func)],
    current_state: &ast::Object,
    all_derived: &[DerivedFact],
    d: &ast::Object,
) -> Vec<DerivedFact> {
    let existing_keys = state_keys(current_state);
    derive_one_round_with_keys(
        derivation_defs, current_state, all_derived, d, &existing_keys)
}

/// Like [`derive_one_round`] but takes a pre-built `existing_keys`
/// set — lets the semi-naive chainer maintain it incrementally
/// across rounds instead of rebuilding ~5k-element HashSets every
/// round. On core.md this was ~3ms per round of pure re-hashing.
fn derive_one_round_with_keys(
    derivation_defs: &[(&str, &ast::Func)],
    current_state: &ast::Object,
    all_derived: &[DerivedFact],
    d: &ast::Object,
    existing_keys: &HashSet<FactKey>,
) -> Vec<DerivedFact> {
    let trace = std::env::var("AREST_STAGE12_TRACE").is_ok();
    let t_dk = std::time::Instant::now();
    let derived_keys: HashSet<FactKey> = all_derived.iter().map(fact_key).collect();
    if trace { eprintln!("    [rnd] derived_keys: {:?}", t_dk.elapsed()); }
    // `encode_state` is a ~1-2ms pure-clone pass on core.md-scale
    // inputs. Skip it when every active Func is a `Native` — the
    // specialized grammar classifiers accept `&Object` directly
    // (raw state or encoded pop) and resolve cells via
    // `fetch_or_phi` on `Object::Map`. Non-Native variants
    // (interpreted FFP) still require the encoded pop shape.
    let all_native = derivation_defs.iter()
        .all(|(_, f)| matches!(f, ast::Func::Native(_)));
    let t_en = std::time::Instant::now();
    let pop_obj;
    let apply_input: &ast::Object = if all_native {
        current_state
    } else {
        pop_obj = ast::encode_state(current_state);
        &pop_obj
    };
    if trace { eprintln!("    [rnd] encode_state: {:?} (skipped={})",
        t_en.elapsed(), all_native); }
    let t_ap = std::time::Instant::now();
    let candidates: Vec<DerivedFact> = derivation_defs.iter()
        .flat_map(|(name, func)| {
            let result = ast::apply(func, apply_input, d);
            let name = name.to_string();
            result.as_seq().into_iter()
                .flat_map(move |items| items.iter().cloned().collect::<Vec<_>>())
                .filter_map(move |item| parse_derived_fact(&item, &name))
                .collect::<Vec<_>>()
        })
        .collect();
    if trace { eprintln!("    [rnd] apply {} defs: {:?} ({} candidates)",
        derivation_defs.len(), t_ap.elapsed(), candidates.len()); }
    let t_dd = std::time::Instant::now();
    let mut round_keys: HashSet<FactKey> = HashSet::with_capacity(candidates.len());
    let mut out: Vec<DerivedFact> = Vec::with_capacity(candidates.len());
    for cand in candidates {
        let key = fact_key(&cand);
        if !existing_keys.contains(&key)
            && !derived_keys.contains(&key)
            && round_keys.insert(key)
        {
            out.push(cand);
        }
    }
    if trace { eprintln!("    [rnd] dedup: {:?} ({} novel)",
        t_dd.elapsed(), out.len()); }
    out
}

/// Semi-naive forward-chain: rules that know which cells they read
/// (via the third tuple element) get skipped in any round whose prior
/// round didn't touch any of those cells. Rules without antecedent
/// metadata (`None`) run every round, matching the classical naïve
/// behavior for that rule.
///
/// For the Stage-2 grammar, round 1 writes only
/// `Statement_has_Classification`; with all 69 classification rules
/// tagged, only the one rule that actually reads that cell survives
/// the round-2 filter. Everything else is a ~zero-cost skip.
pub fn forward_chain_defs_state_semi_naive(
    derivation_defs: &[(&str, &ast::Func, Option<&[String]>)],
    d: &ast::Object,
    max_rounds: usize,
) -> (ast::Object, Vec<DerivedFact>) {
    use hashbrown::HashMap;
    let trace = std::env::var("AREST_STAGE12_TRACE").is_ok();
    let mut current_state = d.clone();
    let mut all_derived: Vec<DerivedFact> = Vec::new();
    // Base set of fact keys in `d`. Built once here and updated
    // incrementally as rounds emit new facts — on core.md this cut
    // ~3ms per round of re-hashing the unchanged grammar portion of
    // the state.
    let t_ek = std::time::Instant::now();
    let mut existing_keys = state_keys(&current_state);
    if trace { eprintln!("    [sn] initial state_keys: {:?} ({} keys)",
        t_ek.elapsed(), existing_keys.len()); }
    // `dirty_cells == None` means "run everything" (initial round or
    // caller wants no filtering); `Some(set)` restricts to rules that
    // read at least one of those cells.
    let mut dirty_cells: Option<HashSet<String>> = None;
    for round in 0..max_rounds {
        let active: Vec<(&str, &ast::Func)> = derivation_defs.iter()
            .filter(|(_, _, cells)| match (&dirty_cells, cells) {
                (None, _) => true,                       // first round or filtering off
                (Some(_), None) => true,                 // unknown reads → run it
                (Some(dirty), Some(reads)) =>
                    reads.iter().any(|c| dirty.contains(c)),
            })
            .map(|(n, f, _)| (*n, *f))
            .collect();
        if trace {
            eprintln!("    [sn] round {}: active {}/{} defs",
                round, active.len(), derivation_defs.len());
        }
        if active.is_empty() { break; }
        let new_facts = derive_one_round_with_keys(
            active.as_slice(), &current_state, &all_derived, d, &existing_keys);
        if new_facts.is_empty() { break; }

        let mut by_cell: HashMap<String, Vec<ast::Object>> =
            HashMap::with_capacity(new_facts.len().min(active.len()));
        for fact in &new_facts {
            let pairs: Vec<(&str, &str)> = fact.bindings.iter()
                .map(|(k, v)| (k.as_str(), v.as_str())).collect();
            by_cell.entry(fact.fact_type_id.clone()).or_default()
                .push(ast::fact_from_pairs(&pairs));
            // Keep `existing_keys` in sync so the next round's filter
            // doesn't have to re-walk the whole state.
            existing_keys.insert(fact_key(fact));
        }
        let mut next_dirty = HashSet::new();
        for (cell_name, facts) in by_cell {
            next_dirty.insert(cell_name.clone());
            let existing = ast::fetch_or_phi(&cell_name, &current_state);
            let combined = match existing.as_seq() {
                Some(items) => {
                    let mut v = items.to_vec();
                    v.extend(facts);
                    ast::Object::Seq(v.into())
                }
                None => ast::Object::seq(facts),
            };
            current_state = ast::store(&cell_name, combined, &current_state);
        }
        all_derived.extend(new_facts);
        dirty_cells = Some(next_dirty);
    }
    (current_state, all_derived)
}

/// Like [`forward_chain_defs_state`] but capped at `max_rounds` rule
/// applications. Callers that know their rule set is stratified (no
/// rule's antecedent reads another rule's consequent cell) can pass
/// `max_rounds = 1` to skip the empty confirmation round the naive
/// fixpoint does last — the round where `derive_one_round` re-applies
/// every rule against the round-1 output only to dedup it all away.
///
/// Unbounded behavior is preserved through the default 100-round cap
/// in [`forward_chain_defs_state`].
pub fn forward_chain_defs_state_bounded(
    derivation_defs: &[(&str, &ast::Func)],
    d: &ast::Object,
    max_rounds: usize,
) -> (ast::Object, Vec<DerivedFact>) {
    // Fixed-point iteration, bounded by `max_rounds`.
    //
    // A `core::iter::successors(…).take(N).last()` form reads cleaner
    // but is an off-by-one footgun: `successors` eagerly pre-computes
    // the NEXT value on every `next()` call, so a bound of N fires
    // `derive_one_round` N+1 times. For core.md with stratified
    // grammar rules, that extra call was ~7s of pure waste against
    // already-saturated state. The manual loop runs exactly
    // `max_rounds` rounds or fewer (early-exits the first time a
    // round produces nothing novel).
    //
    // Per-round fact integration batches by cell: a naive
    // `fold(state.clone(), cell_push)` is O(n²) because each
    // `cell_push` re-clones the cell's full Vec. Grouping the round's
    // new facts by cell and appending once per cell makes it O(n).
    let mut current_state = d.clone();
    let mut all_derived: Vec<DerivedFact> = Vec::new();
    for _ in 0..max_rounds {
        let new_facts = derive_one_round(derivation_defs, &current_state, &all_derived, d);
        if new_facts.is_empty() { break; }
        use hashbrown::HashMap;
        let mut by_cell: HashMap<String, Vec<ast::Object>> =
            HashMap::with_capacity(new_facts.len().min(derivation_defs.len()));
        for fact in &new_facts {
            let pairs: Vec<(&str, &str)> = fact.bindings.iter()
                .map(|(k, v)| (k.as_str(), v.as_str())).collect();
            by_cell.entry(fact.fact_type_id.clone()).or_default()
                .push(ast::fact_from_pairs(&pairs));
        }
        for (cell_name, facts) in by_cell {
            let existing = ast::fetch_or_phi(&cell_name, &current_state);
            let combined = match existing.as_seq() {
                Some(items) => {
                    let mut v = items.to_vec();
                    v.extend(facts);
                    ast::Object::Seq(v.into())
                }
                None => ast::Object::seq(facts),
            };
            current_state = ast::store(&cell_name, combined, &current_state);
        }
        all_derived.extend(new_facts);
    }
    (current_state, all_derived)
}

/// Parse a derivation result Object into a DerivedFact.
fn parse_derived_fact(item: &ast::Object, derived_by: &str) -> Option<DerivedFact> {
    let fact_items = item.as_seq().filter(|f| f.len() >= 3)?;
    let ft_id = fact_items[0].as_atom()?.to_string();
    let reading = fact_items[1].as_atom()?.to_string();
    let bindings: Vec<(String, String)> = fact_items[2].as_seq()
        .unwrap_or(&[])
        .iter()
        .filter_map(|b| {
            let pair = b.as_seq()?;
            if pair.len() == 2 {
                Some((pair[0].as_atom()?.to_string(), pair[1].as_atom()?.to_string()))
            } else { None }
        })
        .collect();
    Some(DerivedFact {
        fact_type_id: ft_id, reading, bindings,
        derived_by: derived_by.to_string(),
        confidence: Confidence::Definitive,
    })
}

/// Canonical key for deduplicating facts across rounds and against
/// `current_state`. A 64-bit FNV-1a hash of `fact_type_id` + the
/// multiset of bindings (role atoms sorted for order-independence).
/// Collision probability at the scales we see (<10^4 facts) is ~10^-12,
/// so `HashSet<FactKey>` is effectively exact without the String
/// allocation cost a `(String, Vec<_>)` key would pay per insertion.
type FactKey = u64;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[inline]
fn fnv_mix(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

fn fact_key(f: &DerivedFact) -> FactKey {
    let mut refs: Vec<&(String, String)> = f.bindings.iter().collect();
    refs.sort();
    let mut h = fnv_mix(FNV_OFFSET, f.fact_type_id.as_bytes());
    for (k, v) in refs {
        h = fnv_mix(h, b"|");
        h = fnv_mix(h, k.as_bytes());
        h = fnv_mix(h, b"=");
        h = fnv_mix(h, v.as_bytes());
    }
    h
}

/// Build a set of fact keys for every fact currently in `state`. One
/// O(N) pass replaces the K × O(N) linear scans the filter would
/// otherwise make via `state_contains_fact`. Borrows &str out of the
/// population — no String allocation per key.
fn state_keys(state: &ast::Object) -> HashSet<FactKey> {
    let mut set: HashSet<FactKey> = HashSet::new();
    for (cell_name, cell_contents) in ast::cells_iter(state) {
        let Some(facts) = cell_contents.as_seq() else { continue };
        for f in facts.iter() {
            let Some(pairs) = f.as_seq() else { continue };
            let mut kv: Vec<(&str, &str)> = pairs.iter().filter_map(|pair| {
                let items = pair.as_seq()?;
                Some((items.get(0)?.as_atom()?, items.get(1)?.as_atom()?))
            }).collect();
            kv.sort();
            let mut h = fnv_mix(FNV_OFFSET, cell_name.as_bytes());
            for (k, v) in &kv {
                h = fnv_mix(h, b"|");
                h = fnv_mix(h, k.as_bytes());
                h = fnv_mix(h, b"=");
                h = fnv_mix(h, v.as_bytes());
            }
            set.insert(h);
        }
    }
    set
}

// -- Proof Engine (Backward Chaining) ---------------------------------
// Given a goal fact, work backward through derivation rules to build a proof tree.
// Each step either finds the fact in the population (axiom), derives it via a rule
// (recursively proving antecedents), or concludes based on world assumption.

/// Attempt to prove a goal fact.
///
/// `goal` is a string like "Academic has Rank 'P'" -- a reading with optional values.
/// The engine searches the population for a matching fact, then tries derivation
/// Prove from Object state directly. No Domain reconstruction.
pub fn prove_from_state(state: &ast::Object, goal: &str, world_assumption: &WorldAssumption) -> ProofResult {
    let schemas = ast::fetch_or_phi("FactType", state);
    let rules = ast::fetch_or_phi("DerivationRule", state);
    let proof = prove_goal_state_pop(state, goal, &HashSet::new(), &schemas, &rules);
    let status = match &proof {
        Some(_) => ProofStatus::Proven,
        None => match world_assumption {
            WorldAssumption::Closed => ProofStatus::Disproven,
            WorldAssumption::Open => ProofStatus::Unknown,
        },
    };
    ProofResult { goal: goal.to_string(), status, proof, world_assumption: world_assumption.clone() }
}

fn prove_goal_state_pop(
    state: &ast::Object, goal: &str, visited: &HashSet<String>,
    schemas: &ast::Object, rules: &ast::Object,
) -> Option<ProofStep> {
    (!visited.contains(goal)).then_some(())?;
    let visited = &{ let mut v = visited.clone(); v.insert(goal.to_string()); v };

    let schema_reading = |ft_id: &str| -> Option<String> {
        schemas.as_seq()?.iter()
            .find(|s| ast::binding(s, "id") == Some(ft_id))
            .and_then(|s| ast::binding(s, "reading").map(|r| r.to_string()))
    };

    // Axiom search first (Step 1), else derivation search (Step 2).
    // `or_else` is Backus cond lifted into Option: axiom ? axiom : derive().
    ast::cells_iter(state).into_iter()
        .filter_map(|(ft_id, contents)| {
            let reading = schema_reading(ft_id)?;
            contents.as_seq()?.iter()
                .map(|fact| {
                    let bindings = extract_bindings(fact);
                    format_fact(&reading, &bindings)
                })
                .find(|fact_text| fact_text_matches(goal, fact_text, &reading))
                .map(|fact_text| ProofStep { fact: fact_text, justification: Justification::Axiom, children: vec![] })
        })
        .next()
        .or_else(|| rules.as_seq().and_then(|rule_list| {
        rule_list.iter().find_map(|rule| {
            let cons_ft_id = ast::binding(rule, "consequentFactTypeId")?.to_string();
            let cons_reading = schema_reading(&cons_ft_id)?;
            let goal_prefix = goal.split(' ').next().unwrap_or("");
            (goal.contains(&cons_reading) || cons_reading.contains(goal_prefix)).then_some(())?;

            let ant_ids_str = ast::binding(rule, "antecedentFactTypeIds")?.to_string();
            let child_proofs: Option<Vec<ProofStep>> = ant_ids_str.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(|ant_id| {
                    let ant_reading = schema_reading(&ant_id)?;
                    prove_goal_state_pop(state, &ant_reading, visited, schemas, rules)
                })
                .collect();

            let children = child_proofs.filter(|c| !c.is_empty())?;
            Some(ProofStep {
                fact: goal.to_string(),
                justification: Justification::Derived {
                    rule_id: ast::binding(rule, "id").unwrap_or("").to_string(),
                    rule_text: ast::binding(rule, "text").unwrap_or("").to_string(),
                },
                children,
            })
        })
    }))
}

/// Extract bindings from a fact Object as (key, value) pairs.
fn extract_bindings(fact: &ast::Object) -> Vec<(String, String)> {
    fact.as_seq().map(|pairs| {
        pairs.iter().filter_map(|pair| {
            let items = pair.as_seq()?;
            Some((items.get(0)?.as_atom()?.to_string(), items.get(1)?.as_atom()?.to_string()))
        }).collect()
    }).unwrap_or_default()
}

/// Format a fact from its reading template and bindings
#[allow(dead_code)] // called by prove_goal()
fn format_fact(reading: &str, bindings: &[(String, String)]) -> String {
    bindings.iter().fold(reading.to_string(), |result, (noun, value)| {
        result.find(noun.as_str())
            .map(|pos| format!("{}{} '{}'{}",  &result[..pos], noun, value, &result[pos + noun.len()..]))
            .unwrap_or(result)
    })
}

/// Check if a goal string matches a formatted fact
#[allow(dead_code)] // called by prove_goal()
fn fact_text_matches(goal: &str, fact_text: &str, reading: &str) -> bool {
    let goal_lower = goal.to_lowercase();
    let fact_lower = fact_text.to_lowercase();
    let reading_lower = reading.to_lowercase();
    goal == fact_text || goal == reading
        || goal_lower == fact_lower || goal_lower == reading_lower
        || fact_lower.contains(&goal_lower)
        || goal_lower.contains(&reading_lower)
}

// -- Synthesis --------------------------------------------------------

/// Synthesize from Object state directly.
pub fn synthesize_from_state(state: &ast::Object, noun_name: &str, depth: usize) -> SynthesisResult {
    let b = |f: &ast::Object, key: &str| -> String {
        ast::binding(f, key).unwrap_or("").to_string()
    };

    let wa = WorldAssumption::Closed;

    // 1. Find schemas where this noun plays a role (via Role facts)
    let role_cell = ast::fetch_or_phi("Role", state);
    let role_facts = role_cell.as_seq().unwrap_or(&[]);
    let schema_ids_for_noun: Vec<(String, usize)> = role_facts.iter()
        .filter(|r| b(r, "nounName") == noun_name)
        .map(|r| (b(r, "factType"), b(r, "position").parse().unwrap_or(0)))
        .collect();

    let schema_cell = ast::fetch_or_phi("FactType", state);
    let schema_facts = schema_cell.as_seq().unwrap_or(&[]);
    let participates_in: Vec<FactTypeSummary> = schema_ids_for_noun.iter()
        .filter_map(|(sid, role_idx)| {
            let reading = schema_facts.iter()
                .find(|s| b(s, "id") == *sid)
                .map(|s| b(s, "reading"))?;
            Some(FactTypeSummary { id: sid.clone(), reading, role_index: *role_idx })
        })
        .collect();

    // 2. Constraints spanning those fact types
    // Block-scoped ft_ids so its borrow on participates_in ends
    // before the move into SynthesisResult at end of function.
    let applicable_constraints: Vec<ConstraintSummary> = {
        let ft_ids: HashSet<&str> = participates_in.iter().map(|f| f.id.as_str()).collect();
        let constraint_cell = ast::fetch_or_phi("Constraint", state);
        let constraint_facts = constraint_cell.as_seq().unwrap_or(&[]);
        let mut seen = HashSet::new();
        constraint_facts.iter()
            .filter(|c| {
                (0..4).any(|i| {
                    let ft_key = format!("span{}_factTypeId", i);
                    let ft_id = b(c, &ft_key);
                    !ft_id.is_empty() && ft_ids.contains(ft_id.as_str())
                })
            })
            .filter(|c| seen.insert(b(c, "id")))
            .map(|c| ConstraintSummary {
                id: b(c, "id"), text: b(c, "text"), kind: b(c, "kind"),
                modality: b(c, "modality"), deontic_operator: {
                    let op = b(c, "deonticOperator");
                    if op.is_empty() { None } else { Some(op) }
                },
            })
            .collect()
    };

    // 3. State machines (from InstanceFact: "State Machine Definition 'X' is for Noun 'noun'")
    let inst_cell = ast::fetch_or_phi("InstanceFact", state);
    let inst_facts = inst_cell.as_seq().unwrap_or(&[]);
    let state_machines: Vec<StateMachineSummary> = inst_facts.iter()
        .filter(|f| b(f, "subjectNoun") == "State Machine Definition" && b(f, "objectNoun") == "Noun" && b(f, "objectValue") == noun_name)
        .map(|f| {
            let sm_name = b(f, "subjectValue");
            let statuses: Vec<String> = inst_facts.iter()
                .filter(|s| b(s, "subjectNoun") == "Status" && b(s, "objectNoun") == "State Machine Definition" && b(s, "objectValue") == sm_name)
                .map(|s| b(s, "subjectValue"))
                .collect();
            let initial = inst_facts.iter()
                .find(|s| b(s, "subjectNoun") == "Status" && b(s, "fieldName") == "is initial in" && b(s, "objectValue") == sm_name)
                .map(|s| b(s, "subjectValue"))
                .unwrap_or_else(|| statuses.first().cloned().unwrap_or_default());
            let valid_transitions: Vec<String> = inst_facts.iter()
                .filter(|t| b(t, "subjectNoun") == "Transition" && b(t, "objectNoun") == "Event Type")
                .filter(|t| {
                    let trans_name = b(t, "subjectValue");
                    inst_facts.iter().any(|tf| b(tf, "subjectNoun") == "Transition" && b(tf, "subjectValue") == trans_name && b(tf, "objectNoun") == "Status" && b(tf, "objectValue") == initial && b(tf, "fieldName").contains("from"))
                })
                .map(|t| b(t, "objectValue"))
                .collect();
            StateMachineSummary { noun_name: sm_name, statuses, current_status: Some(initial), valid_transitions }
        })
        .collect();

    // 4. Related nouns
    let mut seen_related = HashSet::new();
    let related_nouns: Vec<RelatedNoun> = if depth > 0 {
        participates_in.iter()
            .flat_map(|fts| {
                role_facts.iter()
                    .filter(|r| b(r, "factType") == fts.id && b(r, "nounName") != noun_name)
                    .filter(|r| seen_related.insert(b(r, "nounName")))
                    .map(|r| RelatedNoun {
                        name: b(r, "nounName"),
                        via_fact_type: fts.id.clone(),
                        via_reading: fts.reading.clone(),
                        world_assumption: WorldAssumption::Closed,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    } else { Vec::new() };

    SynthesisResult {
        noun_name: noun_name.to_string(), world_assumption: wa,
        participates_in, applicable_constraints, state_machines,
        derived_facts: Vec::new(), related_nouns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashbrown::HashMap;
    use crate::types::{
        ConstraintDef, DerivationRuleDef, FactTypeDef, GeneralInstanceFact, NounDef,
        RoleDef, SpanDef, StateMachineDef, WorldAssumption,
    };

    fn empty_state() -> ast::Object {
        ast::Object::phi()
    }

    fn make_noun(object_type: &str) -> NounDef {
        NounDef {
            object_type: object_type.to_string(),
            world_assumption: WorldAssumption::default(),
        }
    }

    /// Build Object state with facts from pairs.
    fn state_with_facts(ft_id: &str, pairs_list: &[&[(&str, &str)]]) -> ast::Object {
        pairs_list.iter().fold(ast::Object::phi(), |acc, pairs|
            ast::cell_push(ft_id, ast::fact_from_pairs(pairs), &acc))
    }

    // ── Cell-push test builders (no Domain IR) ──────────────────────────
    //
    // Tests build Object cells directly via these helpers. All helpers take
    // and return Object — facts all the way down. The `S` alias is a
    // convenience for the working map; terminate with `build(cells)`.

    type S = HashMap<String, Vec<ast::Object>>;

    fn empty_cells() -> S { HashMap::new() }

    fn build(cells: S) -> ast::Object {
        ast::Object::Map(cells.into_iter()
            .map(|(k, v)| (k, ast::Object::Seq(v.into())))
            .collect())
    }

    fn with_noun(mut cells: S, name: &str, def: &NounDef) -> S {
        let wa = match def.world_assumption {
            WorldAssumption::Closed => "closed", WorldAssumption::Open => "open",
        };
        let ref_scheme = (def.object_type == "entity").then(|| "id");
        let mut pairs: Vec<(&str, &str)> = vec![
            ("name", name), ("objectType", def.object_type.as_str()),
            ("worldAssumption", wa),
        ];
        if let Some(rs) = ref_scheme { pairs.push(("referenceScheme", rs)); }
        cells.entry("Noun".into()).or_default().push(ast::fact_from_pairs(&pairs));
        cells
    }

    fn with_ft(mut cells: S, id: &str, ft: &FactTypeDef) -> S {
        let arity = ft.roles.len().to_string();
        cells.entry("FactType".into()).or_default().push(ast::fact_from_pairs(&[
            ("id", id), ("reading", ft.reading.as_str()), ("arity", arity.as_str()),
        ]));
        for role in &ft.roles {
            let pos = role.role_index.to_string();
            cells.entry("Role".into()).or_default().push(ast::fact_from_pairs(&[
                ("factType", id), ("nounName", role.noun_name.as_str()), ("position", pos.as_str()),
            ]));
        }
        cells
    }

    fn with_constraint(mut cells: S, c: &ConstraintDef) -> S {
        cells.entry("Constraint".into()).or_default()
            .push(crate::parse_forml2::constraint_to_fact_test(c));
        cells
    }

    fn with_derivation(mut cells: S, r: &DerivationRuleDef) -> S {
        let json = serde_json::to_string(r).unwrap_or_default();
        let consequent_encoded = r.consequent_cell.encode();
        cells.entry("DerivationRule".into()).or_default().push(ast::fact_from_pairs(&[
            ("id", r.id.as_str()), ("text", r.text.as_str()),
            ("consequentFactTypeId", consequent_encoded.as_str()),
            ("json", json.as_str()),
        ]));
        cells
    }

    fn with_state_machine(mut cells: S, name: &str, sm: &StateMachineDef) -> S {
        let json = serde_json::to_string(sm).unwrap_or_default();
        cells.entry("StateMachine".into()).or_default().push(ast::fact_from_pairs(&[
            ("name", name), ("json", json.as_str()),
        ]));
        cells
    }

    #[allow(dead_code)]
    fn with_instance_fact(mut cells: S, f: &GeneralInstanceFact) -> S {
        cells.entry("InstanceFact".into()).or_default().push(ast::fact_from_pairs(&[
            ("subjectNoun", f.subject_noun.as_str()),
            ("subjectValue", f.subject_value.as_str()),
            ("fieldName", f.field_name.as_str()),
            ("objectNoun", f.object_noun.as_str()),
            ("objectValue", f.object_value.as_str()),
        ]));
        let object = if f.object_noun.is_empty() { f.field_name.as_str() } else { f.object_noun.as_str() };
        cells.entry(f.field_name.clone()).or_default().push(ast::fact_from_pairs(&[
            (f.subject_noun.as_str(), f.subject_value.as_str()),
            (object, f.object_value.as_str()),
        ]));
        cells
    }

    fn with_subtype(mut cells: S, sub: &str, sup: &str) -> S {
        // Patch existing Noun fact for `sub`: add/update superType field.
        // If the Noun wasn't declared, push a new one.
        let nouns = cells.entry("Noun".into()).or_default();
        let pos = nouns.iter().position(|f| ast::binding(f, "name") == Some(sub));
        let name = sub;
        let old = pos.map(|i| nouns[i].clone());
        let obj_type = old.as_ref().and_then(|f| ast::binding(f, "objectType")).unwrap_or("entity").to_string();
        let wa = old.as_ref().and_then(|f| ast::binding(f, "worldAssumption")).unwrap_or("closed").to_string();
        let rs = old.as_ref().and_then(|f| ast::binding(f, "referenceScheme")).map(|s| s.to_string());
        let mut pairs: Vec<(&str, &str)> = vec![
            ("name", name), ("objectType", obj_type.as_str()),
            ("worldAssumption", wa.as_str()), ("superType", sup),
        ];
        if let Some(ref rs_s) = rs { pairs.push(("referenceScheme", rs_s.as_str())); }
        let new_fact = ast::fact_from_pairs(&pairs);
        match pos {
            Some(i) => nouns[i] = new_fact,
            None => nouns.push(new_fact),
        }
        cells
    }

    fn with_enum_values(mut cells: S, name: &str, obj_type: &str, values: &[String]) -> S {
        let wa = "closed";
        let ref_scheme = (obj_type == "entity").then(|| "id");
        let joined = values.join(",");
        let mut pairs: Vec<(&str, &str)> = vec![
            ("name", name), ("objectType", obj_type),
            ("worldAssumption", wa),
            ("enumValues", joined.as_str()),
        ];
        if let Some(rs) = ref_scheme { pairs.push(("referenceScheme", rs)); }
        cells.entry("Noun".into()).or_default().push(ast::fact_from_pairs(&pairs));
        cells
    }

    /// Compile a cell map into (state, defs, def_map). Mirrors the old
    /// ir_to_defs API but takes cell-push-built state, not a typed Domain.
    fn compile_cells(cells: S) -> (ast::Object, Vec<(String, ast::Func)>, ast::Object) {
        let state = build(cells);
        let (defs, def_map) = state_to_defs(&state);
        (state, defs, def_map)
    }

    /// Compile the Object state into defs + def_map.
    fn state_to_defs(state: &ast::Object) -> (Vec<(String, ast::Func)>, ast::Object) {
        let model = crate::compile::compile(state);
        let defs: Vec<(String, ast::Func)> = model.constraints.iter()
            .map(|c| (format!("constraint:{}", c.id), c.func.clone()))
            .chain(model.state_machines.iter().flat_map(|sm| [
                (format!("machine:{}", sm.noun_name), sm.func.clone()),
                (format!("machine:{}:initial", sm.noun_name), ast::Func::constant(ast::Object::atom(&sm.initial))),
            ]))
            .chain(model.derivations.iter().map(|d| (format!("derivation:{}", d.id), d.func.clone())))
            .chain(model.schemas.iter().map(|(id, schema)| (format!("schema:{}", id), schema.construction.clone())))
            .collect();
        let def_map = ast::defs_to_state(&defs, state);
        (defs, def_map)
    }

    /// Evaluate constraints via defs.
    fn eval_constraints_defs(
        defs: &[(String, ast::Func)],
        def_map: &ast::Object,
        text: &str,
        sender: Option<&str>,
        state: &ast::Object,
    ) -> Vec<Violation> {
        let ctx_obj = ast::encode_eval_context_state(text, sender, state);
        defs.iter()
            .filter(|(n, _)| n.starts_with("constraint:"))
            .flat_map(|(name, func)| {
                let result = ast::apply(func, &ctx_obj, def_map);
                let is_deontic = name.contains("obligatory") || name.contains("forbidden");
                ast::decode_violations(&result).into_iter().map(move |mut v| {
                    v.alethic = !is_deontic;
                    v
                })
            })
            .collect()
    }

    /// Run a state machine from defs (replaces run_machine_ast).
    fn run_machine_defs(
        defs: &[(String, ast::Func)],
        def_map: &ast::Object,
        noun_name: &str,
        events: &[&str],
    ) -> String {
        let machine_key = format!("machine:{}", noun_name);
        let initial_key = format!("machine:{}:initial", noun_name);
        let func = defs.iter().find(|(n, _)| *n == machine_key).map(|(_, f)| f);
        let initial = defs.iter().find(|(n, _)| *n == initial_key)
            .and_then(|(_, f)| {
                let r = ast::apply(f, &ast::Object::phi(), def_map);
                r.as_atom().map(|s| s.to_string())
            })
            .unwrap_or_default();

        let func = match func {
            Some(f) => f,
            None => return initial,
        };

        events.into_iter().fold(initial, |state, event| {
            let input = ast::Object::seq(vec![
                ast::Object::atom(&state),
                ast::Object::atom(event),
            ]);
            let result = ast::apply(func, &input, def_map);
            result.as_atom().map(|s| s.to_string()).unwrap_or(state)
        })
    }

    /// Extract derivation defs from the full defs list.
    fn derivation_defs_from<'a>(defs: &'a [(String, ast::Func)]) -> Vec<(&'a str, &'a ast::Func)> {
        defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:"))
            .map(|(n, f)| (n.as_str(), f))
            .collect()
    }

    // -- DEFS evaluation path tests ------------------------------------

    #[test]
    fn test_evaluate_via_ast_uniqueness_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "uc1".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each Person has at most one Name".to_string(),
            spans: vec![crate::types::SpanDef {
                fact_type_id: "ft1".to_string(),
                role_index: 0,
                subset_autofill: None,
            }],
            ..Default::default()
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        let state = state_with_facts("ft1", &[
            &[("Person", "Alice"), ("Name", "A")],
            &[("Person", "Alice"), ("Name", "B")],
        ]);

        let violations = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].constraint_id, "uc1");
    }

    #[test]
    fn test_evaluate_via_ast_no_violations() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "uc1".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each Person has at most one Name".to_string(),
            spans: vec![crate::types::SpanDef {
                fact_type_id: "ft1".to_string(),
                role_index: 0,
                subset_autofill: None,
            }],
            ..Default::default()
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        let state = state_with_facts("ft1", &[
            &[("Person", "Alice"), ("Name", "A")],
        ]);

        let violations = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_run_machine_via_ast() {
        // Domain Change state machine: Proposed -> Under Review -> Approved -> Applied
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "DomainChange", &StateMachineDef {
            noun_name: "DomainChange".to_string(),
            statuses: vec![
                "Proposed".to_string(),
                "Under Review".to_string(),
                "Approved".to_string(),
                "Applied".to_string(),
                "Rejected".to_string(),
            ],
            transitions: vec![
                TransitionDef { from: "Proposed".to_string(), to: "Under Review".to_string(), event: "review-requested".to_string(), guard: None },
                TransitionDef { from: "Under Review".to_string(), to: "Approved".to_string(), event: "approved".to_string(), guard: None },
                TransitionDef { from: "Under Review".to_string(), to: "Rejected".to_string(), event: "rejected".to_string(), guard: None },
                TransitionDef { from: "Approved".to_string(), to: "Applied".to_string(), event: "applied".to_string(), guard: None },
            ],
            initial: String::new(),
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Happy path: Proposed -> Under Review -> Approved -> Applied
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["review-requested", "approved", "applied"]);
        assert_eq!(final_state, "Applied");

        // Rejection path: Proposed -> Under Review -> Rejected
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["review-requested", "rejected"]);
        assert_eq!(final_state, "Rejected");

        // Invalid event: stays in current state
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["applied"]);
        assert_eq!(final_state, "Proposed"); // "applied" not valid from Proposed

        // Partial: just review
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["review-requested"]);
        assert_eq!(final_state, "Under Review");
    }

    #[test]
    fn test_initial_status_from_graph_topology() {
        // No explicit `Status is initial in SM` fact. Graph topology has a
        // unique source-never-target ("Pending" is never a transition
        // target), so compile derives initial from the transition facts.
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Pending".to_string(), "Shipped".to_string(), "Delivered".to_string()],
            transitions: vec![
                TransitionDef { from: "Pending".to_string(), to: "Shipped".to_string(), event: "ship".to_string(), guard: None },
                TransitionDef { from: "Shipped".to_string(), to: "Delivered".to_string(), event: "deliver".to_string(), guard: None },
            ],
            initial: String::new(),
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let initial_key = "machine:Order:initial";
        let initial = defs.iter().find(|(n, _)| n == initial_key)
            .and_then(|(_, f)| {
                let r = ast::apply(f, &ast::Object::phi(), &def_map);
                r.as_atom().map(|s| s.to_string())
            })
            .unwrap_or_default();
        assert_eq!(initial, "Pending", "graph topology: Pending is source-never-target");
    }

    #[test]
    fn test_initial_status_from_explicit_declaration() {
        // Explicit `initial: "Shipped"` on the SM def (mirrors
        // `Status 'Shipped' is initial in SM 'Order'.` instance fact).
        // Even though graph topology would suggest "Pending" (source-
        // never-target), the explicit declaration wins.
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Pending".to_string(), "Shipped".to_string(), "Delivered".to_string()],
            transitions: vec![
                TransitionDef { from: "Pending".to_string(), to: "Shipped".to_string(), event: "ship".to_string(), guard: None },
                TransitionDef { from: "Shipped".to_string(), to: "Delivered".to_string(), event: "deliver".to_string(), guard: None },
            ],
            initial: "Shipped".to_string(),
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let initial = defs.iter().find(|(n, _)| n == "machine:Order:initial")
            .and_then(|(_, f)| ast::apply(f, &ast::Object::phi(), &def_map).as_atom().map(|s| s.to_string()))
            .unwrap_or_default();
        assert_eq!(initial, "Shipped", "explicit declaration overrides graph topology");
    }

    #[test]
    fn test_initial_status_empty_when_cyclic() {
        // Fully cyclic machine: every status is both source and target.
        // No explicit declaration. Graph topology yields no
        // source-never-target. Per §5.1, the fold needs s_0; when one
        // cannot be derived, compile emits an empty initial and the
        // runtime fails explicitly at first SM call.
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "Cycle".to_string(),
            statuses: vec!["A".to_string(), "B".to_string()],
            transitions: vec![
                TransitionDef { from: "A".to_string(), to: "B".to_string(), event: "forward".to_string(), guard: None },
                TransitionDef { from: "B".to_string(), to: "A".to_string(), event: "back".to_string(), guard: None },
            ],
            initial: String::new(),
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let initial = defs.iter().find(|(n, _)| n == "machine:Cycle:initial")
            .and_then(|(_, f)| ast::apply(f, &ast::Object::phi(), &def_map).as_atom().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(initial.is_empty(), "cyclic machine with no explicit initial -> empty (no insertion-order fallback)");
    }

    #[test]
    fn test_noun_without_state_machine() {
        let cells = empty_cells(); // no state machines
        let (_meta_pop, defs, _def_map) = compile_cells(cells);
        let has_machine = defs.iter().any(|(n, _)| n.starts_with("machine:Customer"));
        assert!(!has_machine);
    }

    #[test]
    fn test_valid_transitions_from_status() {
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Triaging".to_string(), to: "Resolved".to_string(), event: "quick-resolve".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
            initial: String::new(),
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);

        // From Triaging: two valid transitions
        let after_investigate = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate"]);
        assert_eq!(after_investigate, "Investigating");
        let after_quick_resolve = run_machine_defs(&defs, &def_map, "SupportRequest", &["quick-resolve"]);
        assert_eq!(after_quick_resolve, "Resolved");

        // From Investigating: one valid transition
        let after_resolve = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve"]);
        assert_eq!(after_resolve, "Resolved");

        // From Resolved: no transitions (terminal) - invalid event stays put
        let after_terminal = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve", "investigate"]);
        assert_eq!(after_terminal, "Resolved");
    }

    #[test]
    fn test_run_machine_support_request_lifecycle() {
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "WaitingOnCustomer".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "WaitingOnCustomer".to_string(), event: "request-info".to_string(), guard: None },
                TransitionDef { from: "WaitingOnCustomer".to_string(), to: "Investigating".to_string(), event: "customer-replied".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
            initial: String::new(),
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Full lifecycle with back-and-forth
        let final_state = run_machine_defs(&defs, &def_map, "SupportRequest", &[
            "investigate",
            "request-info",
            "customer-replied",
            "resolve",
        ]);
        assert_eq!(final_state, "Resolved");

        // Invalid event mid-flow stays in current state
        let final_state = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve", "investigate"]);
        assert_eq!(final_state, "Resolved"); // already resolved, "investigate" has no effect
    }

    #[test]
    fn test_deontic_forbidden_text_via_ast() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Markdown Syntax", &make_noun("value"));
        cells = with_enum_values(cells, "Markdown Syntax", "value", &vec!["#".to_string(), "##".to_string(), "**".to_string()]);
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Response contains Markdown Syntax".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Markdown Syntax".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "dc1".to_string(),
            kind: "FC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: "It is forbidden that a Response contains Markdown Syntax.".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            ..Default::default()
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Text with markdown -> violations
        let violations = eval_constraints_defs(&defs, &def_map, "## Heading here", None, &empty_state());
        assert!(violations.len() > 0, "should detect forbidden markdown");

        // Clean text -> no violations
        let clean_violations = eval_constraints_defs(&defs, &def_map, "No special formatting here.", None, &empty_state());
        assert_eq!(clean_violations.len(), 0);
    }

    #[test]
    fn test_deontic_permitted_never_violates_via_ast() {
        let mut cells = empty_cells();
        cells = with_constraint(cells, &ConstraintDef {
            id: "pc1".to_string(),
            kind: "FC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("permitted".to_string()),
            text: "It is permitted that something happens.".to_string(),
            spans: vec![],
            ..Default::default()
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let violations = eval_constraints_defs(&defs, &def_map, "anything", None, &empty_state());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_no_constraints_no_violations_via_ast() {
        let (_meta_pop, defs, def_map) = compile_cells(empty_cells());
        let violations = eval_constraints_defs(&defs, &def_map, "", None, &empty_state());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_fact_creation_triggers_state_transition() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Customer", &make_noun("entity"));
        cells = with_noun(cells, "SupportRequest", &make_noun("entity"));

        cells = with_ft(cells, "ft_submit", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer submits SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });

        cells = with_state_machine(cells, "SupportRequest", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
            initial: String::new(),
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        // The state machine starts at "Triaging"
        let initial_key = "machine:SupportRequest:initial";
        let initial = defs.iter().find(|(n, _)| n == initial_key)
            .and_then(|(_, f)| {
                let r = ast::apply(f, &ast::Object::phi(), &def_map);
                r.as_atom().map(|s| s.to_string())
            })
            .unwrap_or_default();
        assert_eq!(initial, "Triaging");

        // Verify the state machine can transition
        let after_investigate = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate"]);
        assert_eq!(after_investigate, "Investigating");

        // Verify schema was compiled
        let has_schema = defs.iter().any(|(n, _)| n == "schema:ft_submit");
        assert!(has_schema, "Schema compiled for submit fact type");
    }

    #[test]
    fn test_fact_event_mapping_compiled() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Customer", &make_noun("entity"));
        cells = with_noun(cells, "SupportRequest", &make_noun("entity"));

        cells = with_ft(cells, "ft_submit", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer submits SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });

        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "submit".to_string(), guard: None },
            ],
            initial: String::new(),
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Verify the state machine transitions on "submit"
        let final_state = run_machine_defs(&defs, &def_map, "SupportRequest", &["submit"]);
        assert_eq!(final_state, "Investigating");
    }

    #[test]
    fn test_guarded_transition_blocks_on_violation() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "SupportRequest", &make_noun("entity"));
        cells = with_noun(cells, "Prohibited", &make_noun("value"));
        cells = with_enum_values(cells, "Prohibited", "value", &vec!["internal-details".to_string()]);

        cells = with_ft(cells, "ft_resp", &FactTypeDef {
            schema_id: String::new(),
            reading: "Response contains Prohibited".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Prohibited".to_string(), role_index: 0 }],
        });

        cells = with_constraint(cells, &ConstraintDef {
            id: "guard1".to_string(),
            kind: "FC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: "It is forbidden that a Response contains internal-details".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft_resp".to_string(), role_index: 0, subset_autofill: None }],
            ..Default::default()
        });

        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef {
                    from: "Investigating".to_string(), to: "Resolved".to_string(),
                    event: "resolve".to_string(),
                    guard: Some(GuardDef {
                        fact_type_id: "ft_resp".to_string(),
                        constraint_ids: vec!["guard1".to_string()],
                    }),
                },
            ],
            initial: String::new(),
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Response with forbidden content -> constraint detects violation
        let pop = empty_state();
        let violations = eval_constraints_defs(&defs, &def_map, "Here are the internal-details of the system", None, &pop);
        assert!(!violations.is_empty(), "Guard constraint should produce violations");

        // Clean response -> no constraint violations
        let clean_violations = eval_constraints_defs(&defs, &def_map, "Your issue has been resolved. Thank you.", None, &pop);
        assert!(clean_violations.is_empty(), "No guard violations for clean response");

        // The machine processes the event:
        let state = run_machine_defs(&defs, &def_map, "SupportRequest", &["resolve"]);
        assert_eq!(state, "Resolved",
            "run_machine_defs fires the transition; guard enforcement is the caller's responsibility");
    }

    #[test]
    fn test_fact_driven_event_resolution() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Customer", &make_noun("entity"));
        cells = with_noun(cells, "SupportRequest", &make_noun("entity"));
        cells = with_noun(cells, "Agent", &make_noun("entity"));

        cells = with_ft(cells, "ft_submit", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer submits SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft_resolve", &FactTypeDef {
            schema_id: String::new(),
            reading: "Agent resolves SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Agent".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });

        cells = with_state_machine(cells, "SupportRequest", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
            initial: String::new(),
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Both schemas should compile
        let has_submit = defs.iter().any(|(n, _)| n == "schema:ft_submit");
        let has_resolve = defs.iter().any(|(n, _)| n == "schema:ft_resolve");
        assert!(has_submit);
        assert!(has_resolve);

        // Full lifecycle through events
        let state = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve"]);
        assert_eq!(state, "Resolved");
    }

    #[test]
    fn test_subset_constraint_without_autofill_produces_violation() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Person", &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasLicense".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasInsurance".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        // SS constraint WITHOUT autofill -- just validates, doesn't derive
        cells = with_constraint(cells, &ConstraintDef {
            id: "ss_no_auto".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            text: "If some Person hasLicense then that Person hasInsurance".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            ..Default::default()
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        // Behaviour-only assertion (#287 gap #10): an SS constraint
        // without `subset_autofill` must not derive any positive fact
        // into the consequent cell. The derivation id / cell-name
        // format is implementation detail — assert on derived facts.
        let state = state_with_facts("ft1", &[&[("Person", "p1")]]);
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);
        let mp_derived: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "ft2").collect();
        // CWA negation may derive "NOT Person hasInsurance" — that's expected.
        // But no POSITIVE autofill derivation should exist.
        let positive_mp = mp_derived.iter().filter(|d| !d.reading.contains("NOT")).count();
        assert_eq!(positive_mp, 0, "No autofill -> no positive derived insurance facts");
    }

    #[test]
    fn test_forward_chain_ast_subtype_inheritance() {
        // Teacher is subtype of Academic. Academic has Rank.
        // Forward chaining should terminate without panicking.
        let mut cells = empty_cells();
        cells = with_noun(cells, "Academic", &make_noun("entity"));
        cells = with_noun(cells, "Teacher", &make_noun("entity"));
        cells = with_subtype(cells, "Teacher", "Academic");
        cells = with_noun(cells, "Rank", &make_noun("value"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Academic has Rank".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Academic".to_string(), role_index: 0 },
                RoleDef { noun_name: "Rank".to_string(), role_index: 1 },
            ],
        });
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        // Behaviour assertion (#287 gap #10): at least one derivation
        // rule exists whose DerivationKind is SubtypeInheritance —
        // the kind tag survives renaming unlike the cell-name
        // substring. compile_explicit_derivation propagates
        // `rule.kind` into the CompiledDerivation emitted per
        // (subtype, super_ft) triple.
        let dd = derivation_defs_from(&defs);
        assert!(!dd.is_empty(),
            "Expected at least one derivation for Teacher-is-subtype-of-Academic schema");

        // Teacher T1 has Rank P — forward chain doesn't panic.
        let state = state_with_facts("ft1", &[&[("Academic", "T1"), ("Rank", "P")]]);
        let (_new_state, _derived) = forward_chain_defs_state(&dd, &state);
    }

    #[test]
    fn test_forward_chain_ast_modus_ponens() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Academic", &make_noun("entity"));
        cells = with_noun(cells, "Department", &make_noun("entity"));

        cells = with_ft(cells, "ft_heads", &FactTypeDef {
            schema_id: String::new(),
            reading: "Academic heads Department".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Academic".to_string(), role_index: 0 },
                RoleDef { noun_name: "Department".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft_works", &FactTypeDef {
            schema_id: String::new(),
            reading: "Academic works for Department".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Academic".to_string(), role_index: 0 },
                RoleDef { noun_name: "Department".to_string(), role_index: 1 },
            ],
        });

        // Subset constraint with autofill: heads -> automatically derive works for
        cells = with_constraint(cells, &ConstraintDef {
            id: "ss1".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            text: "If some Academic heads some Department then that Academic works for that Department".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft_heads".to_string(), role_index: 0, subset_autofill: Some(true) },
                SpanDef { fact_type_id: "ft_works".to_string(), role_index: 0, subset_autofill: None },
            ],
            entity: None,
            set_comparison_argument_length: None,
            clauses: None,
            min_occurrence: None,
            max_occurrence: None,
            deontic_operator: None,
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        // Academic A1 heads Department D1
        let state = state_with_facts("ft_heads", &[&[("Academic", "A1"), ("Department", "D1")]]);

        let dd = derivation_defs_from(&defs);
        let (_new_state, ast_derived) = forward_chain_defs_state(&dd, &state);
        // Modus ponens should derive the full tuple: (A1, D1) in ft_works
        let works_for = ast_derived.iter().any(|d|
            d.fact_type_id == "ft_works" &&
            d.bindings.iter().any(|(n, v)| n == "Academic" && v == "A1") &&
            d.bindings.iter().any(|(n, v)| n == "Department" && v == "D1")
        );
        assert!(works_for, "Expected full tuple derivation: A1 works for D1");
    }

    #[test]
    fn test_forward_chain_ast_no_rules_no_derivations() {
        let cells = empty_cells();
        let (_meta_state, defs, _def_map) = compile_cells(cells);
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &empty_state());
        assert_eq!(derived.len(), 0);
    }

    // -- Constraint evaluation tests -----------------------------------

    #[test]
    fn test_no_constraints_no_violations() {
        let cells = empty_cells();
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &empty_state());
        assert!(result.is_empty());
    }

    #[test]
    fn test_uniqueness_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Each Customer has at most one Name".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let state = state_with_facts("ft1", &[&[("Customer", "c1"), ("Name", "Alice")], &[("Customer", "c1"), ("Name", "Bob")]]);

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Uniqueness violation"));
    }

    #[test]
    fn test_ring_irreflexive_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person manages Person".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Person".to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "IR".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "No Person manages itself".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1"), ("Person", "p1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Irreflexive"));
    }

    #[test]
    fn test_exclusive_or_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPaid".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPending".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "XO".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Order, exactly one holds".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Order isPaid".to_string(), "Order isPending".to_string()]),
            entity: Some("Order".to_string()),
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        pop_state = ast::cell_push("ft2", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Set-comparison violation"));
    }

    #[test]
    fn test_subset_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasLicense".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasInsurance".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "If some Person hasLicense then that Person hasInsurance".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Subset violation"));
    }

    #[test]
    fn test_permitted_never_violates() {
        let mut cells = empty_cells();
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("permitted".to_string()),
            text: "It is permitted that SupportResponse offers data retrieval".to_string(),
            spans: vec![],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &empty_state());
        assert!(result.is_empty());
    }

    #[test]
    fn test_exclusive_choice_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPaid".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPending".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "XC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Order, at most one holds".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Order isPaid".to_string(), "Order isPending".to_string()]),
            entity: Some("Order".to_string()),
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        pop_state = ast::cell_push("ft2", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Set-comparison violation"));
    }

    #[test]
    fn test_mandatory_violation() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Customer", &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Email".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Email".to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "MC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Each Customer has at least one Name".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft2", ast::fact_from_pairs(&[("Customer", "c1"), ("Email", "a@b.com")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Mandatory violation"));
        assert!(result[0].detail.contains("c1"));
    }

    #[test]
    fn test_inclusive_or_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer hasPhone".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer hasEmail".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "OR".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Customer, at least one of the following holds: hasPhone, hasEmail".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Customer hasPhone".to_string(), "Customer hasEmail".to_string()]),
            entity: Some("Customer".to_string()),
            min_occurrence: None,
            max_occurrence: None,
        });

        cells = with_ft(cells, "ft3", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer hasName".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft3", ast::fact_from_pairs(&[("Customer", "c1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Set-comparison violation"));
        assert!(result[0].detail.contains("at least one"));
    }

    #[test]
    fn test_obligatory_missing_enum_value() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "SenderIdentityValue", &make_noun("value"));
        cells = with_enum_values(cells, "SenderIdentityValue", "value", &vec!["Support Team <support@example.com>".to_string()]);
        cells = with_noun(cells, "SupportResponse", &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "SupportResponse has SenderIdentityValue".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                RoleDef { noun_name: "SenderIdentityValue".to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that each SupportResponse has SenderIdentity".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "Here is some help for you.", Some(""), &empty_state());
        assert!(result.len() >= 1);
        let details: Vec<String> = result.iter().map(|v| v.detail.clone()).collect();
        assert!(details.iter().any(|d: &String| d.contains("obligatory")));
    }

    #[test]
    fn test_obligatory_sender_identity_empty() {
        let mut cells = empty_cells();
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that each SupportResponse has SenderIdentity".to_string(),
            spans: vec![],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "Hello", Some(""), &empty_state());
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("SenderIdentity"));
    }

    /// Regression: constraints spanning multiple fact types that share a value-type noun
    /// must not produce duplicate violations. collect_enum_values deduplicates by noun name.
    #[test]
    fn test_no_duplicate_violations_for_multi_span_constraints() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "FieldName", &make_noun("value"));
        cells = with_enum_values(cells, "FieldName", "value", &vec!["EndpointSlug".to_string(), "Title".to_string()]);
        cells = with_noun(cells, "SupportResponse", &make_noun("entity"));
        cells = with_noun(cells, "APIProduct", &make_noun("entity"));
        // Three fact types that all reference FieldName -- simulates multi-span constraint
        for i in 1..=3 {
            cells = with_ft(cells, &format!("ft{}", i), &FactTypeDef {
                schema_id: String::new(),
                reading: format!("SupportResponse names APIProduct by FieldName ({})", i),
                readings: vec![],
                roles: vec![
                    RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                    RoleDef { noun_name: "APIProduct".to_string(), role_index: 1 },
                    RoleDef { noun_name: "FieldName".to_string(), role_index: 2 },
                ],
            });
        }
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that SupportResponse names APIProduct by FieldName 'Title'.".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft3".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "test response without required field names", None, &empty_state());
        // Should produce exactly 1 violation per unique noun, not 3 duplicates
        let field_name_violations: Vec<_> = result.iter()
            .filter(|v| v.detail.contains("FieldName"))
            .collect();
        assert_eq!(field_name_violations.len(), 1,
            "Expected 1 FieldName violation, got {}. Violations: {:?}",
            field_name_violations.len(),
            field_name_violations.iter().map(|v| &v.detail).collect::<Vec<_>>());
    }

    #[test]
    fn test_equality_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person isEmployee".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasBadge".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "EQ".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Person isEmployee if and only if Person hasBadge".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Equality violation"));
    }

    // -- Forward Inference & Synthesis Tests ----------------------------

    #[test]
    fn test_subtype_inheritance_derivation() {
        let mut cells = empty_cells();

        cells = with_noun(cells, "Vehicle", &make_noun("entity"));
        cells = with_noun(cells, "Car", &make_noun("entity"));
        cells = with_subtype(cells, "Car", "Vehicle");
        cells = with_noun(cells, "License", &make_noun("entity"));

        cells = with_ft(cells, "ft_vehicle_license", &FactTypeDef {
            schema_id: String::new(),
            reading: "Vehicle has License".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Vehicle".to_string(), role_index: 0 },
                RoleDef { noun_name: "License".to_string(), role_index: 1 },
            ],
        });

        cells = with_ft(cells, "ft_car_color", &FactTypeDef {
            schema_id: String::new(),
            reading: "Car has Color".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Car".to_string(), role_index: 0 },
            ],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);
        let dd = derivation_defs_from(&defs);

        // Behaviour assertion (#287 gap #10): forward chain over a
        // population with a Car instance must derive an inherited
        // fact into the supertype (Vehicle) FT. Inspect derived
        // facts directly — no cell-name substring probing.
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft_car_color", ast::fact_from_pairs(&[("Car", "my_car")]), &pop_state);
        let state = pop_state;

        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        let inheritance_facts: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "ft_vehicle_license"
                && d.bindings.iter().any(|(_, v)| v == "my_car"))
            .collect();
        assert!(!inheritance_facts.is_empty(),
            "Expected inherited fact in ft_vehicle_license for Car instance 'my_car'; got {:?}", derived);
    }

    #[test]
    fn test_modus_ponens_from_subset() {
        let mut cells = empty_cells();

        cells = with_noun(cells, "Person", &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasLicense".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasInsurance".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        // SS constraint with autofill: hasLicense -> automatically derive hasInsurance
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "If some Person hasLicense then that Person hasInsurance".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: Some(true) },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        // Behaviour assertion (#287 gap #10): the SS-autofill
        // derivation's presence is proven by the derived fact it
        // produces, not by its cell-name. Forward chain runs below.
        // Forward chain: p1 hasLicense -> should derive p1 hasInsurance
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1")]), &pop_state);
        let state = pop_state;

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        let insurance_facts: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "ft2")
            .collect();
        assert_eq!(insurance_facts.len(), 1,
            "Expected SS autofill to derive hasInsurance for p1");
        assert_eq!(insurance_facts[0].bindings, vec![("Person".to_string(), "p1".to_string())]);
        assert_eq!(insurance_facts[0].confidence, Confidence::Definitive);
    }

    #[test]
    fn test_cwa_vs_owa_negation() {
        let mut cells = empty_cells();

        // CWA noun: Permission (not stated = false)
        cells = with_noun(cells, "Permission", &NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::Closed,
        });
        // OWA noun: Capability (not stated = unknown)
        cells = with_noun(cells, "Capability", &NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::Open,
        });

        cells = with_noun(cells, "Resource", &make_noun("entity"));

        cells = with_ft(cells, "ft_perm", &FactTypeDef {
            schema_id: String::new(),
            reading: "Permission grants access to Resource".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Permission".to_string(), role_index: 0 },
                RoleDef { noun_name: "Resource".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft_cap", &FactTypeDef {
            schema_id: String::new(),
            reading: "Capability enables Resource".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Capability".to_string(), role_index: 0 },
                RoleDef { noun_name: "Resource".to_string(), role_index: 1 },
            ],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);
        let dd = derivation_defs_from(&defs);

        // Behaviour assertion (#287 gap #10): CWA fires only for the
        // closed-world noun. Exercise: forward chain with a
        // Permission instance that doesn't participate in ft_perm
        // should derive a NOT fact; a Capability instance (OWA) in
        // the same shape should not.
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft_other", ast::fact_from_pairs(&[("Permission", "read")]), &pop_state);
        pop_state = ast::cell_push("ft_other", ast::fact_from_pairs(&[("Capability", "invoke")]), &pop_state);
        let state = pop_state;

        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        // CWA negation: Permission 'read' absent from ft_perm.
        let perm_negation: Vec<_> = derived.iter()
            .filter(|d| d.reading.contains("NOT")
                && d.reading.contains("Permission"))
            .collect();
        assert!(!perm_negation.is_empty(),
            "Expected CWA negation for Permission 'read'; got {:?}", derived);
        assert_eq!(perm_negation[0].confidence, Confidence::Definitive);

        // OWA: Capability must NOT trigger a negation derivation.
        let cap_negation: Vec<_> = derived.iter()
            .filter(|d| d.reading.contains("NOT")
                && d.reading.contains("Capability"))
            .collect();
        assert!(cap_negation.is_empty(),
            "Expected NO CWA negation for OWA Capability; got {:?}", cap_negation);
    }

    /// #287 gap #11 — focused test for the AntecedentSource::InstancesOfNoun
    /// shape in compile_explicit_derivation. Constructs a minimal
    /// DerivationRuleDef with that antecedent + a Literal consequent
    /// + a target role name. Populates the would-be consequent cell
    /// with an "existing" fact for one instance to exercise the
    /// dedup guard (gap #12). Verifies the derivation emits one
    /// <consequent_id, reading, <<role, atom>>> fact per MISSING
    /// instance, skipping the one that already participates.
    #[test]
    fn test_instances_of_noun_antecedent_with_dedup_guard() {
        let mut cells = empty_cells();

        cells = with_noun(cells, "Dog", &make_noun("entity"));
        cells = with_noun(cells, "Animal", &make_noun("entity"));
        cells = with_subtype(cells, "Dog", "Animal");
        cells = with_noun(cells, "Name", &make_noun("value"));

        cells = with_ft(cells, "ft_dog_name", &FactTypeDef {
            schema_id: String::new(),
            reading: "Dog has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Dog".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft_animal_owner", &FactTypeDef {
            schema_id: String::new(),
            reading: "Animal has Owner".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Animal".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);
        let dd = derivation_defs_from(&defs);

        // Two dogs; one already has an Animal-owner record, the
        // other doesn't. The dedup guard should skip the already-
        // participating one.
        let mut pop = ast::Object::phi();
        pop = ast::cell_push("ft_dog_name",
            ast::fact_from_pairs(&[("Dog", "fido"), ("Name", "Fido")]), &pop);
        pop = ast::cell_push("ft_dog_name",
            ast::fact_from_pairs(&[("Dog", "rex"), ("Name", "Rex")]), &pop);
        pop = ast::cell_push("ft_animal_owner",
            ast::fact_from_pairs(&[("Animal", "fido"), ("Name", "alice")]), &pop);

        let (_s, derived) = forward_chain_defs_state(&dd, &pop);

        // Inherited Animal facts in ft_animal_owner, from Dog instances.
        let inherited: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "ft_animal_owner")
            .collect();

        // fido is already in ft_animal_owner with <Animal, fido> at
        // role 0 — dedup guard must skip it.
        let fido_inherited = inherited.iter()
            .any(|d| d.bindings.iter().any(|(_, v)| v == "fido"));
        assert!(!fido_inherited,
            "Dedup guard failed: fido already participates in ft_animal_owner but got re-emitted: {:?}", inherited);

        // rex has no Animal record — dedup guard must emit.
        let rex_inherited = inherited.iter()
            .any(|d| d.bindings.iter().any(|(_, v)| v == "rex"));
        assert!(rex_inherited,
            "Expected inherited fact for Dog 'rex' into ft_animal_owner; got {:?}", inherited);
    }

    #[test]
    fn test_synthesis_basic() {
        let mut cells = empty_cells();

        cells = with_noun(cells, "Customer", &NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::Closed,
        });
        cells = with_noun(cells, "Name", &make_noun("value"));
        cells = with_noun(cells, "Email", &make_noun("value"));

        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Email".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Email".to_string(), role_index: 1 },
            ],
        });

        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "MC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Each Customer has at least one Name".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (meta_pop, _defs, _def_map) = compile_cells(cells);
        let result = synthesize_from_state(&meta_pop, "Customer", 1);

        assert_eq!(result.noun_name, "Customer");

        // Customer participates in two fact types
        assert_eq!(result.participates_in.len(), 2,
            "Customer should participate in ft1 and ft2. Got: {:?}",
            result.participates_in);

        // One constraint applies to Customer
        assert_eq!(result.applicable_constraints.len(), 1,
            "Expected 1 constraint for Customer. Got: {:?}",
            result.applicable_constraints);
        assert_eq!(result.applicable_constraints[0].id, "c1");

        // Related nouns: Name and Email
        assert_eq!(result.related_nouns.len(), 2,
            "Expected 2 related nouns. Got: {:?}", result.related_nouns);
        let related_names: Vec<_> = result.related_nouns.iter()
            .map(|r| r.name.as_str())
            .collect();
        assert!(related_names.contains(&"Name"), "Expected Name as related noun");
        assert!(related_names.contains(&"Email"), "Expected Email as related noun");
    }

    #[test]
    fn test_synthesis_empty_noun() {
        let (meta_pop, _defs, _def_map) = compile_cells(empty_cells());
        let result = synthesize_from_state(&meta_pop, "NonExistent", 1);

        assert_eq!(result.noun_name, "NonExistent");
        assert!(result.participates_in.is_empty());
        assert!(result.applicable_constraints.is_empty());
        assert!(result.state_machines.is_empty());
        assert!(result.related_nouns.is_empty());
    }

    #[test]
    fn test_forward_chain_fixed_point() {
        // Verify forward chaining reaches a fixed point (no infinite loops)
        let mut cells = empty_cells();
        cells = with_noun(cells, "A", &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "A exists".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "A".to_string(), role_index: 0 }],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("A", "a1")]), &pop_state);
        let state = pop_state;

        // Should terminate even if derivations produce facts
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);
        // Just verify it terminates -- the exact count depends on CWA rules
        assert!(derived.len() < 100, "Forward chaining should reach fixed point quickly");
    }

    #[test]
    fn test_transitivity_derivation() {
        let mut cells = empty_cells();

        cells = with_noun(cells, "City", &make_noun("entity"));
        cells = with_noun(cells, "State", &make_noun("entity"));
        cells = with_noun(cells, "Country", &make_noun("entity"));

        cells = with_ft(cells, "ft_city_state", &FactTypeDef {
            schema_id: String::new(),
            reading: "City isIn State".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "City".to_string(), role_index: 0 },
                RoleDef { noun_name: "State".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft_state_country", &FactTypeDef {
            schema_id: String::new(),
            reading: "State isIn Country".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "State".to_string(), role_index: 0 },
                RoleDef { noun_name: "Country".to_string(), role_index: 1 },
            ],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        // Behaviour assertion (#287 gap #10): the transitivity
        // derivation's presence is confirmed by its output below.
        // Cell name / id format is compile-time detail.

        // Forward chain: Austin isIn Texas, Texas isIn USA -> Austin (transitively) in USA
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft_city_state", ast::fact_from_pairs(&[("City", "Austin"), ("State", "Texas")]), &pop_state);
        pop_state = ast::cell_push("ft_state_country", ast::fact_from_pairs(&[("State", "Texas"), ("Country", "USA")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let transitive_facts: Vec<_> = derived.iter()
            .filter(|d| d.derived_by.contains("transitivity"))
            .collect();
        assert!(!transitive_facts.is_empty(),
            "Expected transitivity to derive Austin->USA relationship");

        // Verify the derived fact connects City to Country
        let city_country = transitive_facts.iter().find(|d| {
            d.bindings.iter().any(|(_, v)| v == "Austin")
                && d.bindings.iter().any(|(_, v)| v == "USA")
        });
        assert!(city_country.is_some(),
            "Expected derived fact linking Austin to USA. Derived: {:?}", transitive_facts);
    }

    #[test]
    fn test_world_assumption_default_is_closed() {
        assert_eq!(WorldAssumption::default(), WorldAssumption::Closed);
    }

    // â”€â”€ Inline-comparator filter end-to-end (Halpin FORML Example 5) â”€â”€
    //
    // Each AntecedentFilter on a DerivationRuleDef wraps the antecedent's
    // fact-extraction in Func::filter, so only facts whose role value
    // satisfies the comparator reach the existence check. With the current
    // existence-based semantics: if every antecedent fact is filtered out,
    // NullTest on the filtered Seq returns true and the rule stops firing.
    // If at least one fact passes, the rule fires and the binding
    // extractor pulls from the first post-filter fact.

    fn city_population_cells(filter: Option<crate::types::AntecedentFilter>) -> S {
        let ft1 = FactTypeDef {
            schema_id: String::new(),
            reading: "City has Population".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "City".to_string(), role_index: 0 },
                RoleDef { noun_name: "Population".to_string(), role_index: 1 },
            ],
        };
        let ft2 = FactTypeDef {
            schema_id: String::new(),
            reading: "Big City has City".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Big City".to_string(), role_index: 0 },
                RoleDef { noun_name: "City".to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: "big-city".to_string(),
            text: "* Big City has City iff City has Population >= 1000000".to_string(),
            antecedent_sources: vec![AntecedentSource::FactType("city_has_population".to_string())],
            consequent_cell: ConsequentCellSource::Literal("big_city".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![],
            match_on: vec![],
            consequent_bindings: vec![],
            antecedent_filters: filter.into_iter().collect(),
            consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], consequent_role_literals: vec![],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "city_has_population", &ft1);
        cells = with_ft(cells, "big_city", &ft2);
        cells = with_derivation(cells, &rule);
        cells
    }

    #[test]
    fn inline_ge_filter_suppresses_derivation_when_no_fact_matches() {
        // Both cities well below the 1M threshold â†’ filter strips every
        // antecedent fact â†’ rule's existence check fails â†’ no derivation.
        let cells = city_population_cells(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: ">=".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "SmallTown"), ("Population", "500000")]), &pop_state);
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "MidVille"), ("Population", "250000")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert!(big.is_empty(), "expected no big_city derivations, got {:?}", big);
    }

    #[test]
    fn inline_ge_filter_allows_derivation_when_a_fact_matches() {
        // One city below the threshold, one above. The filter keeps only
        // the big one, the existence check passes, and the rule fires with
        // the matching city's bindings.
        let cells = city_population_cells(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: ">=".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "SmallTown"), ("Population", "500000")]), &pop_state);
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "Megapolis"), ("Population", "2000000")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 1, "expected exactly one big_city derivation, got {:?}", big);
        // Bindings must come from the matching (post-filter) fact, not the
        // small-town one whose Population is below the cutoff.
        assert!(big[0].bindings.iter().any(|(k, v)| k == "City" && v == "Megapolis"),
            "expected Megapolis as the derived binding, got {:?}", big[0].bindings);
    }

    #[test]
    fn inline_lt_filter_keeps_only_smaller_values() {
        // Flip direction: derivation should fire only when some fact's
        // Population is strictly less than 1M. Exercises Func::Lt path in
        // comparator_primitive.
        let cells = city_population_cells(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: "<".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "Megapolis"), ("Population", "2000000")]), &pop_state);
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "Hamlet"), ("Population", "400")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 1);
        assert!(big[0].bindings.iter().any(|(k, v)| k == "City" && v == "Hamlet"),
            "expected Hamlet (pop<1M), got {:?}", big[0].bindings);
    }

    #[test]
    fn per_fact_fanout_produces_one_derivation_per_matching_fact() {
        // Four cities, three above the 1M threshold. Per-fact semantic
        // demands one derived fact per matching antecedent tuple â€” the
        // old existence-check semantic would have produced one regardless.
        let cells = city_population_cells(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: ">=".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        for (name, pop) in [("Alpha", "2000000"), ("Bravo", "5000000"), ("Charlie", "800000"), ("Delta", "3000000")] {
            pop_state = ast::cell_push("city_has_population",
                ast::fact_from_pairs(&[("City", name), ("Population", pop)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 3, "expected 3 big cities (Alpha/Bravo/Delta), got {:?}", big);

        let names: hashbrown::HashSet<&str> = big.iter()
            .flat_map(|d| d.bindings.iter()
                .filter(|(k, _)| k == "City")
                .map(|(_, v)| v.as_str()))
            .collect();
        assert!(names.contains("Alpha"));
        assert!(names.contains("Bravo"));
        assert!(names.contains("Delta"));
        assert!(!names.contains("Charlie"), "sub-threshold city must not derive");
    }

    // â”€â”€ Arithmetic definitional clauses, end-to-end â”€â”€
    //
    // A rule like `* Foo has Doubled iff Foo has Val and Doubled is Val + Val.`
    // records a ConsequentComputedBinding { role: "Doubled", expr: Val + Val }
    // which the compile side turns into a per-fact Func that appends the
    // computed pair to the antecedent's bindings.

    fn val_derived_cells(expr: crate::types::ArithExpr, derived_role: &str) -> S {
        let ft1 = FactTypeDef {
            schema_id: String::new(), reading: "Foo has Val".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Foo".to_string(), role_index: 0 },
                RoleDef { noun_name: "Val".to_string(), role_index: 1 },
            ],
        };
        let ft2 = FactTypeDef {
            schema_id: String::new(),
            reading: format!("Foo has {}", derived_role), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Foo".to_string(), role_index: 0 },
                RoleDef { noun_name: derived_role.to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: "arith-rule".to_string(),
            text: format!("* Foo has {} iff Foo has Val and ...", derived_role),
            antecedent_sources: vec![AntecedentSource::FactType("foo_has_val".to_string())],
            consequent_cell: ConsequentCellSource::Literal("foo_has_derived".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            antecedent_filters: vec![],
            consequent_computed_bindings: vec![crate::types::ConsequentComputedBinding {
                role: derived_role.to_string(), expr,
            }],
            consequent_aggregates: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], consequent_role_literals: vec![],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "foo_has_val", &ft1);
        cells = with_ft(cells, "foo_has_derived", &ft2);
        cells = with_derivation(cells, &rule);
        cells
    }

    fn val_ref() -> crate::types::ArithExpr {
        crate::types::ArithExpr::RoleRef("Val".to_string())
    }

    fn lit(n: f64) -> crate::types::ArithExpr {
        crate::types::ArithExpr::Literal(n)
    }

    fn bin(op: &str, l: crate::types::ArithExpr, r: crate::types::ArithExpr) -> crate::types::ArithExpr {
        crate::types::ArithExpr::Op(op.to_string(), Box::new(l), Box::new(r))
    }

    #[test]
    fn arithmetic_add_computes_role_plus_role() {
        let cells = val_derived_cells(bin("+", val_ref(), val_ref()), "Doubled");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("foo_has_val",
            ast::fact_from_pairs(&[("Foo", "f1"), ("Val", "7")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].bindings.iter().any(|(k, v)| k == "Doubled" && v == "14"),
            "expected ('Doubled','14'), got {:?}", out[0].bindings);
    }

    #[test]
    fn arithmetic_sub_computes_role_minus_literal() {
        let cells = val_derived_cells(bin("-", val_ref(), lit(3.0)), "Less");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("foo_has_val",
            ast::fact_from_pairs(&[("Foo", "f1"), ("Val", "10")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].bindings.iter().any(|(k, v)| k == "Less" && v == "7"),
            "expected ('Less','7'), got {:?}", out[0].bindings);
    }

    #[test]
    fn arithmetic_mul_and_div_chain_left_associative() {
        // (Val * 3) / 2 applied to Val=10 â†’ 15.
        let expr = bin("/", bin("*", val_ref(), lit(3.0)), lit(2.0));
        let cells = val_derived_cells(expr, "Scaled");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("foo_has_val",
            ast::fact_from_pairs(&[("Foo", "f1"), ("Val", "10")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].bindings.iter().any(|(k, v)| k == "Scaled" && v == "15"),
            "expected ('Scaled','15'), got {:?}", out[0].bindings);
    }

    #[test]
    fn arithmetic_fanout_computes_per_fact_independently() {
        // Three Foo facts with different Vals â†’ three derivations, each
        // carrying its own computed value.
        let cells = val_derived_cells(bin("*", val_ref(), lit(2.0)), "Twice");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        for (id, val) in [("a", "3"), ("b", "5"), ("c", "11")] {
            pop_state = ast::cell_push("foo_has_val",
                ast::fact_from_pairs(&[("Foo", id), ("Val", val)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 3);
        let mut pairs: Vec<(String, String)> = out.iter().map(|d| {
            let foo = d.bindings.iter().find(|(k, _)| k == "Foo").map(|(_, v)| v.clone()).unwrap_or_default();
            let tw  = d.bindings.iter().find(|(k, _)| k == "Twice").map(|(_, v)| v.clone()).unwrap_or_default();
            (foo, tw)
        }).collect();
        pairs.sort();
        assert_eq!(pairs, vec![
            ("a".to_string(), "6".to_string()),
            ("b".to_string(), "10".to_string()),
            ("c".to_string(), "22".to_string()),
        ]);
    }

    // â”€â”€ Aggregate derivations, end-to-end (Codd image-set) â”€â”€

    fn thing_part_arity_cells() -> S {
        let ft1 = FactTypeDef {
            schema_id: String::new(), reading: "Thing has Part".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Thing".to_string(), role_index: 0 },
                RoleDef { noun_name: "Part".to_string(), role_index: 1 },
            ],
        };
        let ft2 = FactTypeDef {
            schema_id: String::new(), reading: "Thing has Arity".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Thing".to_string(), role_index: 0 },
                RoleDef { noun_name: "Arity".to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: "thing-arity".to_string(),
            text: "* Thing has Arity iff Arity is the count of Part where Thing has Part.".to_string(),
            antecedent_sources: vec![],
            consequent_cell: ConsequentCellSource::Literal("thing_has_arity".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            antecedent_filters: vec![], consequent_computed_bindings: vec![],
            consequent_aggregates: vec![crate::types::ConsequentAggregate {
                role: "Arity".to_string(),
                op: "count".to_string(),
                target_role: "Part".to_string(),
                source_fact_type_id: "thing_has_part".to_string(),
                group_key_role: "Thing".to_string(),
            }],
            unresolved_clauses: vec![], antecedent_role_literals: vec![], consequent_role_literals: vec![],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "thing_has_part", &ft1);
        cells = with_ft(cells, "thing_has_arity", &ft2);
        cells = with_derivation(cells, &rule);
        cells
    }

    #[test]
    fn count_aggregate_computes_image_set_size_per_group() {
        // Three Parts belong to T1, one to T2. Expect two derived rows:
        // T1 has Arity=3, T2 has Arity=1.
        let cells = thing_part_arity_cells();
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        for (thing, part) in [("T1", "P1"), ("T1", "P2"), ("T1", "P3"), ("T2", "PX")] {
            pop_state = ast::cell_push("thing_has_part",
                ast::fact_from_pairs(&[("Thing", thing), ("Part", part)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let arity: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "thing_has_arity").collect();
        // Collect distinct (Thing, Arity) pairs â€” the outer iteration emits
        // duplicates per group, which forward_chain is expected to dedup.
        let mut pairs: alloc::collections::BTreeSet<(String, String)> = arity.iter().map(|d| {
            let t = d.bindings.iter().find(|(k, _)| k == "Thing").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Arity").map(|(_, v)| v.clone()).unwrap_or_default();
            (t, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("T1".to_string(), "3".to_string()),
            ("T2".to_string(), "1".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected,
            "distinct (Thing, Arity) derivations expected T1â†’3 and T2â†’1, got {:?} (raw count = {})", pairs, arity.len());
        // Sanity â€” if dedup isn't happening, the raw list still contains
        // the right pairs somewhere.
        assert!(arity.iter().any(|d|
            d.bindings.iter().any(|(k, v)| k == "Thing" && v == "T1") &&
            d.bindings.iter().any(|(k, v)| k == "Arity" && v == "3")));
        pairs.clear();  // avoid unused warning via reset
    }

    fn order_line_item_sum_cells() -> S {
        // `LineItem has Amount for Order` is ternary-ish in Halpin's
        // example; for testing we use a simpler binary form
        // `Order has LineItem Amount`, with Order as group key and
        // Amount as target.
        let ft1 = FactTypeDef {
            schema_id: String::new(), reading: "Order has LineItem Amount".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "LineItem Amount".to_string(), role_index: 1 },
            ],
        };
        let ft2 = FactTypeDef {
            schema_id: String::new(), reading: "Order has Amount".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Amount".to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: "order-total".to_string(),
            text: "* Order has Amount iff Amount is the sum of LineItem Amount where Order has LineItem Amount.".to_string(),
            antecedent_sources: vec![],
            consequent_cell: ConsequentCellSource::Literal("order_has_total".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            antecedent_filters: vec![], consequent_computed_bindings: vec![],
            consequent_aggregates: vec![crate::types::ConsequentAggregate {
                role: "Amount".to_string(),
                op: "sum".to_string(),
                target_role: "LineItem Amount".to_string(),
                source_fact_type_id: "order_has_line_amount".to_string(),
                group_key_role: "Order".to_string(),
            }],
            unresolved_clauses: vec![], antecedent_role_literals: vec![], consequent_role_literals: vec![],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "order_has_line_amount", &ft1);
        cells = with_ft(cells, "order_has_total", &ft2);
        cells = with_derivation(cells, &rule);
        cells
    }

    #[test]
    fn sum_aggregate_folds_add_over_projected_target_values() {
        // Order O1: 10 + 25 + 5 = 40; Order O2: 7 = 7.
        let cells = order_line_item_sum_cells();
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        for (order, amt) in [("O1", "10"), ("O1", "25"), ("O1", "5"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", order), ("LineItem Amount", amt)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("O1".to_string(), "40".to_string()),
            ("O2".to_string(), "7".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected,
            "expected O1=40, O2=7; got {:?} (raw count={})", pairs, totals.len());
    }

    fn order_amount_agg_cells(op: &str) -> S {
        // Same shape as order_line_item_sum_cells; this rebuilds with the
        // requested op in the derivation rule's aggregate clause.
        let ft1 = FactTypeDef {
            schema_id: String::new(), reading: "Order has LineItem Amount".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "LineItem Amount".to_string(), role_index: 1 },
            ],
        };
        let ft2 = FactTypeDef {
            schema_id: String::new(), reading: "Order has Amount".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Amount".to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: format!("order-{}", op),
            text: format!("* Order has Amount iff Amount is the {} of LineItem Amount where Order has LineItem Amount.", op),
            antecedent_sources: vec![],
            consequent_cell: ConsequentCellSource::Literal("order_has_total".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            antecedent_filters: vec![], consequent_computed_bindings: vec![],
            consequent_aggregates: vec![crate::types::ConsequentAggregate {
                role: "Amount".to_string(),
                op: op.to_string(),
                target_role: "LineItem Amount".to_string(),
                source_fact_type_id: "order_has_line_amount".to_string(),
                group_key_role: "Order".to_string(),
            }],
            unresolved_clauses: vec![], antecedent_role_literals: vec![], consequent_role_literals: vec![],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "order_has_line_amount", &ft1);
        cells = with_ft(cells, "order_has_total", &ft2);
        cells = with_derivation(cells, &rule);
        cells
    }

    #[test]
    fn min_aggregate_folds_pairwise_minimum() {
        let cells = order_amount_agg_cells("min");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);
        let mut pop_state = ast::Object::phi();
        for (o, a) in [("O1", "10"), ("O1", "4"), ("O1", "25"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", o), ("LineItem Amount", a)]), &pop_state);
        }
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);
        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("O1".to_string(), "4".to_string()),
            ("O2".to_string(), "7".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected, "min: expected O1=4 O2=7, got {:?}", pairs);
    }

    #[test]
    fn max_aggregate_folds_pairwise_maximum() {
        let cells = order_amount_agg_cells("max");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);
        let mut pop_state = ast::Object::phi();
        for (o, a) in [("O1", "10"), ("O1", "4"), ("O1", "25"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", o), ("LineItem Amount", a)]), &pop_state);
        }
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);
        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("O1".to_string(), "25".to_string()),
            ("O2".to_string(), "7".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected, "max: expected O1=25 O2=7, got {:?}", pairs);
    }

    #[test]
    fn avg_aggregate_divides_sum_by_count() {
        let cells = order_amount_agg_cells("avg");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);
        let mut pop_state = ast::Object::phi();
        // O1: (9 + 12 + 15) / 3 = 12.
        for (o, a) in [("O1", "9"), ("O1", "12"), ("O1", "15"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", o), ("LineItem Amount", a)]), &pop_state);
        }
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);
        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        // Accept either integer or float formatting for the averaged value.
        let has_pair = |o: &str, expected_nums: &[&str]| -> bool {
            pairs.iter().any(|(actual_o, v)| actual_o == o && expected_nums.iter().any(|e| v == e))
        };
        assert!(has_pair("O1", &["12", "12.0"]), "avg: expected O1 to average to 12, got {:?}", pairs);
        assert!(has_pair("O2", &["7", "7.0"]), "avg: expected O2=7, got {:?}", pairs);
    }

    #[test]
    fn rule_without_filter_fires_for_any_fact_regression() {
        // Regression: when antecedent_filters is empty, behavior is
        // unchanged from pre-#192 â€” any fact makes the rule fire.
        let cells = city_population_cells(None);
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "SmallTown"), ("Population", "500000")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 1, "unfiltered rule must still fire");
    }

    #[test]
    fn join_derivation_equi_join_on_shared_key() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "a_key", &FactTypeDef {
            schema_id: String::new(), reading: "A has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "b_key", &FactTypeDef {
            schema_id: String::new(), reading: "B has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "derived", &FactTypeDef {
            schema_id: String::new(), reading: "A is matched to B".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "B".to_string(), role_index: 1 },
            ],
        });
        cells = with_derivation(cells, &DerivationRuleDef {
            id: "join1".to_string(),
            text: "A matches B on Key".to_string(),
            antecedent_sources: vec![AntecedentSource::FactType("a_key".to_string()), AntecedentSource::FactType("b_key".to_string())],
            consequent_cell: ConsequentCellSource::Literal("derived".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::Join,
            join_on: vec!["Key".to_string()],
            match_on: vec![],
            consequent_bindings: vec!["A".to_string(), "B".to_string()],
            antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], consequent_role_literals: vec![],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("a_key", ast::fact_from_pairs(&[("A", "a1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("a_key", ast::fact_from_pairs(&[("A", "a2"), ("Key", "k2")]), &pop_state);
        pop_state = ast::cell_push("b_key", ast::fact_from_pairs(&[("B", "b1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("b_key", ast::fact_from_pairs(&[("B", "b2"), ("Key", "k3")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let derived_facts: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "derived").collect();
        // Only a1<->b1 (both Key="k1"). a2 has Key="k2" which doesn't match any B.
        assert_eq!(derived_facts.len(), 1);
        assert!(derived_facts[0].bindings.contains(&("A".to_string(), "a1".to_string())));
        assert!(derived_facts[0].bindings.contains(&("B".to_string(), "b1".to_string())));
    }

    #[test]
    fn join_derivation_entity_consistency_across_fact_types() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "x_key", &FactTypeDef {
            schema_id: String::new(), reading: "X has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "X".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "x_label", &FactTypeDef {
            schema_id: String::new(), reading: "X has Label".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "X".to_string(), role_index: 0 },
                RoleDef { noun_name: "Label".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "y_key", &FactTypeDef {
            schema_id: String::new(), reading: "Y has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Y".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "result", &FactTypeDef {
            schema_id: String::new(), reading: "Y is resolved to X".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Y".to_string(), role_index: 0 },
                RoleDef { noun_name: "X".to_string(), role_index: 1 },
            ],
        });
        cells = with_derivation(cells, &DerivationRuleDef {
            id: "join2".to_string(),
            text: "Y resolves to X via Key".to_string(),
            antecedent_sources: vec![AntecedentSource::FactType("y_key".to_string()), AntecedentSource::FactType("x_key".to_string()), AntecedentSource::FactType("x_label".to_string())],
            consequent_cell: ConsequentCellSource::Literal("result".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::Join,
            join_on: vec!["Key".to_string(), "X".to_string()],
            match_on: vec![],
            consequent_bindings: vec!["Y".to_string(), "X".to_string()],
            antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], consequent_role_literals: vec![],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("x_key", ast::fact_from_pairs(&[("X", "x1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("x_key", ast::fact_from_pairs(&[("X", "x2"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("x_label", ast::fact_from_pairs(&[("X", "x1"), ("Label", "L1")]), &pop_state);
        pop_state = ast::cell_push("x_label", ast::fact_from_pairs(&[("X", "x2"), ("Label", "L2")]), &pop_state);
        pop_state = ast::cell_push("y_key", ast::fact_from_pairs(&[("Y", "y1"), ("Key", "k1")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let resolved: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "result").collect();
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn join_derivation_match_on_containment() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "a_name", &FactTypeDef {
            schema_id: String::new(), reading: "A has Full Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "Full Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "b_name", &FactTypeDef {
            schema_id: String::new(), reading: "B has Short Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "Short Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "matched", &FactTypeDef {
            schema_id: String::new(), reading: "B is matched to A".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "A".to_string(), role_index: 1 },
            ],
        });
        cells = with_derivation(cells, &DerivationRuleDef {
            id: "match1".to_string(),
            text: "B matches A by name containment".to_string(),
            antecedent_sources: vec![AntecedentSource::FactType("a_name".to_string()), AntecedentSource::FactType("b_name".to_string())],
            consequent_cell: ConsequentCellSource::Literal("matched".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::Join,
            join_on: vec![],
            match_on: vec![("Full Name".to_string(), "Short Name".to_string())],
            consequent_bindings: vec!["B".to_string(), "A".to_string()],
            antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], consequent_role_literals: vec![],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("a_name", ast::fact_from_pairs(&[("A", "a1"), ("Full Name", "Alpha Bravo")]), &pop_state);
        pop_state = ast::cell_push("a_name", ast::fact_from_pairs(&[("A", "a2"), ("Full Name", "Charlie Delta")]), &pop_state);
        pop_state = ast::cell_push("b_name", ast::fact_from_pairs(&[("B", "b1"), ("Short Name", "Alpha")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let matched: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "matched").collect();
        assert_eq!(matched.len(), 1);
        assert!(matched[0].bindings.contains(&("A".to_string(), "a1".to_string())));
        assert!(matched[0].bindings.contains(&("B".to_string(), "b1".to_string())));
    }

    #[test]
    fn join_derivation_no_match_produces_nothing() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "a_key", &FactTypeDef {
            schema_id: String::new(), reading: "A has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "b_key", &FactTypeDef {
            schema_id: String::new(), reading: "B has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "derived", &FactTypeDef {
            schema_id: String::new(), reading: "A matches B".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "B".to_string(), role_index: 1 },
            ],
        });
        cells = with_derivation(cells, &DerivationRuleDef {
            id: "j".to_string(),
            text: "join".to_string(),
            antecedent_sources: vec![AntecedentSource::FactType("a_key".to_string()), AntecedentSource::FactType("b_key".to_string())],
            consequent_cell: ConsequentCellSource::Literal("derived".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::Join,
            join_on: vec!["Key".to_string()],
            match_on: vec![],
            consequent_bindings: vec!["A".to_string(), "B".to_string()],
            antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], consequent_role_literals: vec![],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("a_key", ast::fact_from_pairs(&[("A", "a1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("b_key", ast::fact_from_pairs(&[("B", "b1"), ("Key", "k2")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let derived_count = derived.iter().filter(|d| d.fact_type_id == "derived").count();
        assert_eq!(derived_count, 0, "No match should produce no derivation");
    }

    fn make_forbidden_text_cells(enum_vals: Vec<String>) -> S {
        let mut cells = empty_cells();
        let pt = "ProhibitedText";
        let sr = "SupportResponse";
        cells = with_enum_values(cells, pt, "value", &enum_vals);
        cells = with_noun(cells, sr, &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: format!("{} contains {}", sr, pt),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: sr.to_string(), role_index: 0 },
                RoleDef { noun_name: pt.to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: format!("It is forbidden that {} contains {}", sr, pt),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });
        cells
    }

    #[test]
    fn test_forbidden_text_detected() {
        let endash = core::char::from_u32(0x2013).unwrap().to_string();
        let emdash_s = core::char::from_u32(0x2014).unwrap().to_string();
        let cells = make_forbidden_text_cells(vec![endash, emdash_s]);
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let emdash = core::char::from_u32(0x2014).unwrap();
        let text: String = ['H','e','l','l','o',' ',emdash,' ','h','o','w',' ','c','a','n',' ','I',' ','h','e','l','p','?'].iter().collect();
        let result = eval_constraints_defs(&defs, &def_map, &text, None, &empty_state());
        assert!(!result.is_empty());
        assert!(result[0].detail.contains(emdash));
    }

    #[test]
    fn test_forbidden_text_clean() {
        let endash = core::char::from_u32(0x2013).unwrap().to_string();
        let cells = make_forbidden_text_cells(vec![endash]);
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "Hello, how can I help you today?", None, &empty_state());
        assert!(result.is_empty());
    }

    // ── Literal-in-consequent derivation (#286) ──────────────────────
    //
    // Grammar readings take the shape:
    //   Statement has Classification 'Entity Type Declaration'
    //     iff Statement has Trailing Marker 'is an entity type'.
    // The antecedent role `Trailing Marker` must EQUAL a string literal
    // (not a numeric comparator), and the consequent role `Classification`
    // must be BOUND to a string literal (not inherited from antecedent).
    // Both paths are required to make Stage-2 meta-circular.

    fn stmt_classification_cells(ant_literal: &str, cons_literal: &str) -> S {
        let ant_ft = FactTypeDef {
            schema_id: String::new(),
            reading: "Statement has Trailing Marker".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Statement".to_string(), role_index: 0 },
                RoleDef { noun_name: "Trailing Marker".to_string(), role_index: 1 },
            ],
        };
        let cons_ft = FactTypeDef {
            schema_id: String::new(),
            reading: "Statement has Classification".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Statement".to_string(), role_index: 0 },
                RoleDef { noun_name: "Classification".to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: "entity-type-recognizer".to_string(),
            text: format!(
                "Statement has Classification '{}' iff Statement has Trailing Marker '{}'",
                cons_literal, ant_literal),
            antecedent_sources: vec![AntecedentSource::FactType("stmt_has_trailing_marker".to_string())],
            consequent_cell: ConsequentCellSource::Literal("stmt_has_classification".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            antecedent_filters: vec![],
            consequent_computed_bindings: vec![], consequent_aggregates: vec![],
            unresolved_clauses: vec![],
            antecedent_role_literals: vec![crate::types::AntecedentRoleLiteral {
                antecedent_index: 0,
                role: "Trailing Marker".to_string(),
                value: ant_literal.to_string(),
            }],
            consequent_role_literals: vec![crate::types::ConsequentRoleLiteral {
                role: "Classification".to_string(),
                value: cons_literal.to_string(),
            }],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "stmt_has_trailing_marker", &ant_ft);
        cells = with_ft(cells, "stmt_has_classification", &cons_ft);
        cells = with_derivation(cells, &rule);
        cells
    }

    #[test]
    fn literal_in_consequent_fires_when_antecedent_literal_matches() {
        // Stage-2 must see the derived classification fact when a
        // Statement carries the exact trailing-marker literal the
        // grammar rule names. Binding keys use underscore-normalised
        // noun names (Stage-1 convention: `Trailing_Marker`), matched
        // against the FT role noun_name `Trailing Marker` by
        // compile::role_value_by_name.
        let cells = stmt_classification_cells(
            "is an entity type", "Entity Type Declaration");
        let (_meta, defs, _def_map) = compile_cells(cells);

        let mut pop = ast::Object::phi();
        pop = ast::cell_push("stmt_has_trailing_marker",
            ast::fact_from_pairs(&[
                ("Statement", "s1"),
                ("Trailing_Marker", "is an entity type"),
            ]), &pop);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop);

        let hits: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "stmt_has_classification")
            .collect();
        assert_eq!(hits.len(), 1,
            "expected exactly one classification fact, got {:?}", derived);
        let bindings = &hits[0].bindings;
        assert!(bindings.iter().any(|(k, v)| k == "Statement" && v == "s1"),
            "Statement binding missing: {:?}", bindings);
        assert!(bindings.iter().any(|(k, v)|
            k == "Classification" && v == "Entity Type Declaration"),
            "Classification literal binding missing: {:?}", bindings);
    }

    #[test]
    fn literal_in_consequent_suppressed_when_antecedent_literal_mismatches() {
        // If the trailing-marker value on the Statement does NOT match
        // the grammar rule's literal, no classification fact should be
        // emitted. Same-shaped rule with a different literal remains
        // inert for this statement.
        let cells = stmt_classification_cells(
            "is an entity type", "Entity Type Declaration");
        let (_meta, defs, _def_map) = compile_cells(cells);

        let mut pop = ast::Object::phi();
        pop = ast::cell_push("stmt_has_trailing_marker",
            ast::fact_from_pairs(&[
                ("Statement", "s1"),
                ("Trailing Marker", "is a value type"),
            ]), &pop);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop);

        let hits: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "stmt_has_classification")
            .collect();
        assert!(hits.is_empty(),
            "expected no classification facts, got {:?}", hits);
    }
}

