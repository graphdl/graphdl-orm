# Native rule execution — the seconds-class lever (#20/#35, #4 of the mining plan)

2026-07-12. The read path is native (query fix aa68be8e); the compile wall is the
INTERPRETED reduction of rule bodies in op_run_rules' derive fixpoint (3,943 s on
tasks-evt). Per docs/2026-07-10-old-engine-mining.md, seconds-class = NATIVE-FIRST:
each rule body lowered ONCE to a native program run OUTSIDE mu. This spec makes that
executable using the certified-twin pattern already proven three times today
(FastStore, theta arms, query fix): recognize a fixed canon SHAPE, execute it
natively, keep it byte-identical to the interpreted canon behind a kill switch.

## The shape to recognize (arest.canon:838-887, system:compile_rule)

A compiled rule body is ALWAYS this structure, parameterized by three operands:

    compile_rule = COMP( Project[headCols=N2],
                         INSERT(Filter[guard=N1]) ∘ apndr∘⟨atoms=N3, ...⟩ ∘
                         WHILE( not∘null over the remaining atoms:
                                COND(first? NatJoin : JoinOn[on shared cols]) ∘
                                CONS(acc, FetchPop(next atom population)) )
                         FetchPop(first atom population) )

Read plainly, a rule is a conjunctive query:
  1. FetchPop each body atom's population from D (native, fast — the query fix).
  2. Fold them left-to-right: the first atom seeds; each next atom NatJoins (equi-join
     on role-1) or JoinOn-joins (on the shared-variable column pairs the canon threads
     as N(3),N(1),N(3)) onto the accumulator.
  3. Filter the joined tuples by the guard predicate N1 (theta:Filter).
  4. Project to the head columns N2 (theta:Project), yielding the derived head rows.

neval_rule (main.rs:2406) currently reduces this whole term through NEval.mu — the
per-node COMP/CONS/WHILE metacomposition IS the 3,943 s cost. The theta arms
(b88457b8) native-armed NatJoin + 5 helpers but still run under the mu WHILE loop.

## The native executor (what to build)

A function `native_rule_query(ev, cells_index, rid_term) -> Option<Vec<V>>` that:
- MATCHES the compile_rule shape on the rule's built term (exactly as natjoin_config
  matches theta:NatJoin's term — dump one live from the engine first, pin the template,
  reject anything not shape-exact so unknown bodies fall back to mu). Extract the three
  operands: headCols (N2), guard (N1), atom list (N3 — each a FetchPop name + its
  JoinOn column selectors).
- EXECUTES natively, no mu:
  * FetchPop each atom population via the native cells_of first-match (the query fix's
    lookup) — O(1)-ish per atom, not an interpreted D walk.
  * Join with the native hash-join already written for the theta NatJoin arm
    (order-preserving buckets, n_eq-strict keys, A-major emission) — extend it to
    JoinOn (join on the extracted column pairs, emit r ++ s[keep]).
  * Filter: evaluate the guard predicate per row. The guard is itself a small canon
    term (eq/comparison over columns); reuse the theta:Filter arm, OR — if the guard
    is a recognizable compare shape — a native predicate. Rows that fail drop.
  * Project: select headCols from each surviving tuple (native index gather + dedup,
    matching theta:Project's dedup).
  Return the projected head rows.
- Returns None (fall back to neval_rule/mu) for any body that doesn't match the shape
  exactly (aggregates via compile_agg_rule, negation via compile_rule_neg, class rules,
  and any future body form) — those keep the interpreted path until separately armed.

## Wiring in op_run_rules (main.rs ~3623-3710)

In the semi-naive loop's `full` and delta-`cand` closures, BEFORE calling neval_rule,
try native_rule_query for the rule's term. The SEMI-NAIVE contract is unchanged: for
delta rounds, the native path takes the same ⟨sorted delta rows, D⟩ operand (join the
delta variant of the hit atom against D) — a native join over the delta rows instead
of the mu one. Round order, Store prepend order, and the dedup/merge (main.rs:3711+)
stay byte-identical because the native path returns the SAME rows in the SAME order the
mu path's ordered emission produces. α-parallelism (#25, mining #1) then par_iters the
native join's build/probe rows (fuel-gated, ordered collect) once N is Send — a
follow-on multiplier, not required for correctness.

## Certification (the non-negotiable, same as theta arms / query fix)

- Kill switch AREST_NO_NATIVE_RULES=1 forces neval_rule/mu (the differential oracle).
- Property tests: for a battery of real rule bodies (fixture, kernel, sherlock, tasks
  rule set), native vs mu must produce byte-identical head rows AND byte-identical
  final stores. Reuse the stage-e-run.ps1 / fleet-twin-cert.ps1 zero-python byte
  differential over the banked corpora — the store dump must stay byte-clean.
- Gate: tasks-evt wall time native-rules-on vs the 3,943 s baseline, byte-clean first.

## Effort and expected payoff

High effort but BOUNDED by the fixed shape: it is one shape-recognizer + a native
relational pipeline reusing the join/filter/project arms already partly built. It is
NOT a general canon compiler — unmatched bodies fall back to mu, so it lands
incrementally and safely (rule_if bodies first, the dominant shape; agg/neg/class
later). This is the transplant the gate analysis, the mining doc, and the op_run_rules
source all converge on as THE seconds-class lever; rayon (#25) multiplies whatever it
leaves. Start on the rule_if shape (system:compile_rule), which the tasks corpus's
derived facts (Task_is_epic/_blocks_Task/_Priority) are built from.
