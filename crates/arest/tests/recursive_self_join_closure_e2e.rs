// crates/arest/tests/recursive_self_join_closure_e2e.rs
//
// arc Issue 17A / L3 — transitive closure over a SAME-NOUN recursive
// self-join. The arc-agi-3 planning-as-derivation approach builds a
// reachability relation as the least fixed point of a recursive rule.
//
// THE BUG (directed-ring-dedup): three keying layers — the chain's
// `fact_key` / `state_keys` novelty check and the cell-storage
// `synthesize_fact_id` — canonicalized a fact by sorting its
// (role, value) bindings by the FULL TUPLE. For a same-noun ring FT
// (`Glyph reaches Glyph`) both role names are equal, so the sort ordered
// the two bindings by VALUE, collapsing the directed pair <g2,g0> onto
// <g0,g1>'s unordered twin <g0,g2>. A ring cell could then hold only ONE
// direction per unordered pair, so a transitive closure lost its
// wrap-around / reverse edges and never reached the fixpoint. The fix
// sorts by ROLE NAME only (stable), preserving the role-index order of
// same-noun bindings so a->b and b->a key distinctly.
//
// SURFACE FORM: distinct same-type role-players are kept apart with
// Halpin numeric SUBSCRIPTS (`Glyph1`, `Glyph2`, `Glyph3`) — the engine's
// existing rule-local alias mechanism (different subscript = distinct
// entity). The `the <Noun>` head convention does NOT generalise to two
// distinct same-type consequent roles: bare `Glyph` and `the Glyph`
// tokenise identically, so `compute_ring_join_plan` mints a spurious
// equi-join unifying them (subject == object), which empties a recursive
// rule. Multi-distinct-same-type rules must use subscripts.

use arest::ast;

/// Build the reach reading over a caller-supplied `rotates` edge set and
/// derivation rules block, forward-chain to fixed point, and return the
/// ordered `(subject, object)` pairs of `Glyph_reaches_Glyph`.
fn reach_closure_pairs(
    rotate_edges: &[(&str, &str)],
    rules_block: &str,
) -> std::collections::BTreeSet<(String, String)> {
    let mut src = String::from(
        "Glyph(.id) is an entity type.\n\
         Glyph rotates to Glyph.\n\
         Glyph reaches Glyph.\n",
    );
    for (a, b) in rotate_edges {
        src.push_str(&format!("Glyph '{a}' rotates to Glyph '{b}'.\n"));
    }
    src.push_str("\n## Derivation Rules\n");
    src.push_str(rules_block);
    src.push('\n');

    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(&src)
        .expect("parse must succeed");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // Harness guard: the base `rotates` facts must have loaded.
    let rotate_count = ast::fetch_cell_seq("Glyph_rotates_to_Glyph", &d)
        .as_seq()
        .map(|f| f.len())
        .unwrap_or(0);
    assert_eq!(
        rotate_count,
        rotate_edges.len(),
        "harness: all {} base `rotates` facts must load before forward-chaining; \
         got {rotate_count} (parse/load issue, not the closure bug)",
        rotate_edges.len()
    );

    let derivation_refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    assert!(
        !derivation_refs_owned.is_empty(),
        "harness: authored derivation rules must compile into `derivation:*` defs"
    );
    let derivation_refs: Vec<(&str, &ast::Func)> = derivation_refs_owned
        .iter()
        .map(|(n, f)| (n.as_str(), f))
        .collect();

    let (new_d, _derived) = arest::evaluate::forward_chain_defs_state(&derivation_refs, &d);

    ast::fetch_cell_seq("Glyph_reaches_Glyph", &new_d)
        .as_seq()
        .map(|facts| {
            facts
                .iter()
                .filter_map(|f| {
                    let roles = f.as_seq()?;
                    let subj = roles.first()?.as_seq()?.get(1)?.as_atom()?.to_string();
                    let obj = roles.get(1)?.as_seq()?.get(1)?.as_atom()?.to_string();
                    Some((subj, obj))
                })
                .collect()
        })
        .unwrap_or_default()
}

const REACH_RULES: &str = "\
    * Glyph1 reaches Glyph2 iff Glyph1 rotates to Glyph2.\n\
    * Glyph1 reaches Glyph2 iff Glyph1 rotates to Glyph3 and Glyph3 reaches Glyph2.";

fn pairs(es: &[(&str, &str)]) -> std::collections::BTreeSet<(String, String)> {
    es.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

/// 4-cycle g0->g1->g2->g3->g0. Every glyph reaches every glyph (the cycle
/// closes), so the closure is ALL 16 ordered pairs — INCLUDING reflexives
/// (e.g. g0 reaches g0 via the full loop). This is the arc acceptance
/// test: 16 = correct, 8 = collapse, 4 = no-closure.
#[test]
fn four_cycle_reach_closure_builds_all_sixteen_pairs() {
    let got = reach_closure_pairs(
        &[("g0", "g1"), ("g1", "g2"), ("g2", "g3"), ("g3", "g0")],
        REACH_RULES,
    );
    let glyphs = ["g0", "g1", "g2", "g3"];
    let want: std::collections::BTreeSet<(String, String)> = glyphs
        .iter()
        .flat_map(|a| glyphs.iter().map(move |b| (a.to_string(), b.to_string())))
        .collect();
    assert_eq!(
        got, want,
        "4-cycle reach closure must be ALL 16 ordered pairs; got {} pairs: {:?}",
        got.len(),
        got
    );
}

/// 4-node PATH g0->g1->g2->g3 (a DAG, no cycle). The closure is exactly
/// the 6 forward pairs and NOTHING reversed or reflexive. This is the
/// directedness guard: a "fix" that merely unions all pairs (or treats
/// the ring as symmetric) would over-produce reverse/reflexive edges and
/// fail here, even though it would pass the fully-connected cycle test.
#[test]
fn four_node_path_reach_closure_is_directed_and_acyclic() {
    let got = reach_closure_pairs(
        &[("g0", "g1"), ("g1", "g2"), ("g2", "g3")],
        REACH_RULES,
    );
    let want = pairs(&[
        ("g0", "g1"),
        ("g0", "g2"),
        ("g0", "g3"),
        ("g1", "g2"),
        ("g1", "g3"),
        ("g2", "g3"),
    ]);
    assert_eq!(
        got, want,
        "path-graph reach closure must be exactly the 6 forward pairs (no reverse, \
         no reflexive); got {} pairs: {:?}",
        got.len(),
        got
    );
}
