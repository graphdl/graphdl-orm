// crates/arest/tests/aggregate_min_over_recursive_closure_e2e.rs
//
// arc blocker (task arc-min-aggregate-ivm-misfold) — a `min` aggregate over a
// RECURSIVE closure whose recursive step SUMS costs (3-role `Count plus Count
// is Count`) misfolds: it does not fold to the single minimum per group.
//
// ROOT CAUSE — CORRECTED after investigation (see the task's Ticks; the earlier
// "append-only keying" framing this header carried was WRONG). Verified via
// diagnostics:
//   * The min FOLD itself is CORRECT over a COMPLETE source: a STATIC/base (Seq)
//     `reaches` of {2,3} folds to {2}.
//   * The bug is the aggregate's GROUP-FOLD over a DERIVED (Map-stored, growing)
//     source: fresh, BOTH the naive and the deployed semi-naive chains yield
//     shortest(a,c)={3}; the corrected value (2) is NEVER produced. So keying the
//     head is NOT the fix — there is nothing to replace the 3 with.
//   * Reconciles every datum: static/base→correct; arc-gen `steps to` (monotonic,
//     min discovered first)→correct; arc-cost-gen's persisted {2,3} is most
//     likely accumulated stale-db rows, not the fresh symptom.
//   * Ruled out by code-reading: source-encoding (encode_state/encode_state_indexed
//     flatten Map→Seq correctly). The defect is a RUNTIME interaction in the
//     aggregate's group/project/fold over a derived source — pinning the exact
//     line needs live Func-interpreter instrumentation (a focused session).
//
// IGNORED until the fix lands. The test asserts the CORRECT result (min folds to
// {2}); un-ignore when arc-min-aggregate-ivm-misfold is fixed.

use arest::ast;

/// Costs carried by a ternary `Node _ Node at Cost` cell for a (from,to) pair.
fn costs_for(d: &ast::Object, cell: &str, from: &str, to: &str)
    -> std::collections::BTreeSet<String>
{
    ast::fetch_cell_seq(cell, d)
        .as_seq()
        .map(|facts| {
            facts
                .iter()
                .filter_map(|f| {
                    let roles = f.as_seq()?;
                    let subj = roles.first()?.as_seq()?.get(1)?.as_atom()?;
                    let obj = roles.get(1)?.as_seq()?.get(1)?.as_atom()?;
                    let cost = roles.get(2)?.as_seq()?.get(1)?.as_atom()?;
                    (subj == from && obj == to).then(|| cost.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Forward-chain the cost-closure readings to fixed point; return the final
/// state so a test can inspect both `reaches` (source) and `shortest` (head).
fn chain() -> ast::Object {
    let src = "\
        Node(.id) is an entity type.\n\
        Cost(.id) is an entity type.\n\
        \n\
        Node moves to Node at Cost.\n\
        Node reaches Node at Cost. *\n\
        Node shortest reaches Node at Cost. *\n\
        Cost plus Cost is Cost.\n\
        \n\
        ## Derivation Rules\n\
        * Node1 reaches Node2 at Cost1 iff Node1 moves to Node2 at Cost1.\n\
        * Node1 reaches Node2 at Cost3 iff Node1 moves to Node3 at Cost1 and Node3 reaches Node2 at Cost2 and Cost1 plus Cost2 is Cost3.\n\
        * Node1 shortest reaches Node2 at Cost iff Cost is the min of Cost2 where Node1 reaches Node2 at Cost2.\n\
        \n\
        ## Instance Facts\n\
        Node 'a' moves to Node 'b' at Cost '1'.\n\
        Node 'b' moves to Node 'c' at Cost '1'.\n\
        Node 'a' moves to Node 'c' at Cost '3'.\n\
        Cost '1' plus Cost '1' is Cost '2'.\n";

    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

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

    arest::evaluate::forward_chain_defs_state(&derivation_refs, &d).0
}

/// Source-FT resolution under a same-signature collision (the real root cause):
/// the aggregate `min of Cost2 where Node1 reaches Node2 at Cost2` must fold
/// `reaches`, not its same-{Node,Node,Cost}-signature sibling `moves`. Uses a
/// NON-recursive `reaches` (derived from two base relations) so the recursive
/// view-recursion (issue17b) doesn't confound — this isolates the fix.
#[test]
fn aggregate_min_resolves_named_source_not_same_signature_sibling() {
    let src = "\
        Node(.id) is an entity type.\n\
        Cost(.id) is an entity type.\n\
        Node moves to Node at Cost.\n\
        Node hops to Node at Cost.\n\
        Node reaches Node at Cost. *\n\
        Node shortest reaches Node at Cost. *\n\
        \n\
        ## Derivation Rules\n\
        * Node1 reaches Node2 at Cost1 iff Node1 moves to Node2 at Cost1.\n\
        * Node1 reaches Node2 at Cost1 iff Node1 hops to Node2 at Cost1.\n\
        * Node1 shortest reaches Node2 at Cost iff Cost is the min of Cost2 where Node1 reaches Node2 at Cost2.\n\
        \n\
        ## Instance Facts\n\
        Node 'a' moves to Node 'c' at Cost '3'.\n\
        Node 'a' hops to Node 'c' at Cost '2'.\n";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src).expect("parse");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    let owned: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, c)| (n.to_string(), ast::metacompose(c, &d))).collect();
    let refs: Vec<(&str, &ast::Func)> = owned.iter().map(|(n, f)| (n.as_str(), f)).collect();
    let (nd, _) = arest::evaluate::forward_chain_defs_state(&refs, &d);
    // reaches(a,c) = {2 (hops), 3 (moves)}; min MUST be {2}, not min(moves)={3}.
    assert_eq!(
        costs_for(&nd, "Node_shortest_reaches_Node_at_Cost", "a", "c"),
        ["2".to_string()].into_iter().collect(),
        "min must fold the NAMED source `reaches` ({{2,3}}->2), not its \
         same-signature sibling `moves` ({{3}}->3)"
    );
}

/// agg-min-recursive-supersede DISCRIMINATOR: a 3-role group (Node,Node,Feature)
/// like arc-cost-gen, but NON-recursive (single round). If this folds to {2} the
/// 3-role composite group is fine and bug #2 is the recursive/multi-round IVM;
/// if it yields {2,3} the composite group itself is the culprit.
#[test]
fn aggregate_min_three_role_group_nonrecursive_folds_to_min() {
    let src = "\
        Node(.id) is an entity type.\n\
        Cost(.id) is an entity type.\n\
        Feature(.id) is an entity type.\n\
        Node moves to Node for Feature at Cost.\n\
        Node hops to Node for Feature at Cost.\n\
        Node reaches Node for Feature at Cost. *\n\
        Node shortest reaches Node for Feature at Cost. *\n\
        \n\
        ## Derivation Rules\n\
        * Node1 reaches Node2 for Feature1 at Cost1 iff Node1 moves to Node2 for Feature1 at Cost1.\n\
        * Node1 reaches Node2 for Feature1 at Cost1 iff Node1 hops to Node2 for Feature1 at Cost1.\n\
        * Node1 shortest reaches Node2 for Feature1 at Cost iff Cost is the min of Cost2 where Node1 reaches Node2 for Feature1 at Cost2.\n\
        \n\
        ## Instance Facts\n\
        Node 'a' moves to Node 'c' for Feature 'loc' at Cost '3'.\n\
        Node 'a' hops to Node 'c' for Feature 'loc' at Cost '2'.\n";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src).expect("parse");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    let owned: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, c)| (n.to_string(), ast::metacompose(c, &d))).collect();
    let refs: Vec<(&str, &ast::Func)> = owned.iter().map(|(n, f)| (n.as_str(), f)).collect();
    let (nd, _) = arest::evaluate::forward_chain_defs_state(&refs, &d);
    let costs: std::collections::BTreeSet<String> =
        ast::fetch_cell_seq("Node_shortest_reaches_Node_for_Feature_at_Cost", &nd)
            .as_seq().map(|facts| facts.iter().filter_map(|f| {
                let r = f.as_seq()?;
                let n1 = r.first()?.as_seq()?.get(1)?.as_atom()?;
                let n2 = r.get(1)?.as_seq()?.get(1)?.as_atom()?;
                let cost = r.get(3)?.as_seq()?.get(1)?.as_atom()?;
                (n1 == "a" && n2 == "c").then(|| cost.to_string())
            }).collect()).unwrap_or_default();
    assert_eq!(costs, ["2".to_string()].into_iter().collect(),
        "3-role-group non-recursive min must fold to {{2}}; if {{2,3}} the composite \
         3-role group is the bug, if {{2}} bug-#2 is the recursive/multi-round IVM");
}

/// The multi-path group (a,c): direct cost 3 vs a->b->c cost 2. `reaches`
/// (the source) must carry BOTH (precondition); `min` MUST fold to exactly {2}.
#[test]
#[ignore = "arc-min-aggregate-ivm-misfold: aggregate heads are append-only; min over a plus-closure does not fold to the per-group minimum. Un-ignore when keyed-upsert aggregate heads land."]
fn min_over_cost_summing_closure_folds_to_single_minimum() {
    let d = chain();

    // Precondition: the cost-summing closure derives BOTH paths for (a,c).
    let reaches = costs_for(&d, "Node_reaches_Node_at_Cost", "a", "c");
    let want_reaches: std::collections::BTreeSet<String> =
        ["2".to_string(), "3".to_string()].into_iter().collect();
    assert_eq!(
        reaches, want_reaches,
        "precondition: reaches(a,c) must be {{2,3}} (direct 3 + a->b->c 1+1=2); \
         got {:?}. If this fails the closure/`plus` join didn't derive the cheap \
         path and the min test below would be vacuous.",
        reaches
    );

    // The aggregate must fold to exactly the minimum.
    let shortest = costs_for(&d, "Node_shortest_reaches_Node_at_Cost", "a", "c");
    let want: std::collections::BTreeSet<String> = ["2".to_string()].into_iter().collect();
    assert_eq!(
        shortest, want,
        "min `shortest reaches(a,c)` must fold to exactly the minimum {{2}}; \
         got {:?} — the aggregate head did not yield the per-group minimum \
         (append-only head, no group keying)",
        shortest
    );
}

/// Single-path groups already fold correctly even today — a guard so the fix
/// is shown to preserve the working case, not just repair the broken one.
#[test]
#[ignore = "arc-min-aggregate-ivm-misfold: un-ignore with the fix (single-path control)."]
fn min_over_single_path_group_is_unaffected() {
    let d = chain();
    assert_eq!(
        costs_for(&d, "Node_shortest_reaches_Node_at_Cost", "a", "b"),
        ["1".to_string()].into_iter().collect(),
        "single-path (a,b) shortest cost must be {{1}}"
    );
    assert_eq!(
        costs_for(&d, "Node_shortest_reaches_Node_at_Cost", "b", "c"),
        ["1".to_string()].into_iter().collect(),
        "single-path (b,c) shortest cost must be {{1}}"
    );
}
