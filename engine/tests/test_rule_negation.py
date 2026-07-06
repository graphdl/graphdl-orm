"""Stratified negation in rule bodies: the corpus's 'no <clause> [where
<clause>...]' group is a negated EXISTENTIAL — the variable 'no X' introduces is
FRESH (it shadows any outer X), the where-chain scopes INSIDE the negation (it
must never escape as a top-level conjunct), and the group compiles to an
anti-join against the running tuple's shared bound columns. The base's own
rooted-status rule is the canonical case: a Status some Transition is from, with
NO transition to it in the same machine (state.md; the old engine's seed branch
computed this non-monotonic gate in code)."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system


ROOTED = """Status is a value type.
State Machine Definition is an entity type.
Transition is an entity type.
Transition is defined in State Machine Definition.
Transition is from Status.
Transition is to Status.
Status is rooted in State Machine Definition.

* Status is rooted in State Machine Definition iff some Transition is defined in that State Machine Definition and that Transition is from that Status and no Transition is defined in that State Machine Definition where that Transition is to that Status.

Transition 't1' is defined in State Machine Definition 'M'.
Transition 't1' is from Status 'a'.
Transition 't1' is to Status 'b'.
Transition 't2' is defined in State Machine Definition 'M'.
Transition 't2' is from Status 'b'.
Transition 't2' is to Status 'c'.
Transition 't3' is defined in State Machine Definition 'N'.
Transition 't3' is from Status 'b'.
Transition 't3' is to Status 'a'.
"""


def _derive(model):
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    return system.run_rules(D), rep


def _store(head, rows, D):
    from pyarest import ast
    from pyarest.lam import to_lam
    from pyarest.reduce import apply as _ap
    return _ap(ast.Store(head), system._S(to_lam(rows), D))


def test_the_rooted_rule_computes_the_source_statuses():
    D, rep = _derive(ROOTED)
    assert rep["rule_diagnostics"] == []
    got = {tuple(r) for r in system._pop_rows(
        D, "Status_is_rooted_in_State_Machine_Definition")}
    # in M: a is from t1 and nothing in M points to a -> rooted; b is from t2
    # but t1 points to b -> not rooted. In N: b is from t3, nothing in N points
    # to b -> rooted (t1 points to b only in M — the negation is PER MACHINE,
    # which is exactly what the where-scoping preserves).
    assert got == {("a", "M"), ("b", "N")}


def test_at_most_zero_is_negation_spelled_as_frequency():
    # the claude corpus's zero-supplying idiom (affect-select.md, the ledger's
    # own count-of-empty lesson): 'X is Yed by at most 0 Z' is a negated
    # existential — the anti-join with the counted type as the FRESH subject —
    # and the head literal supplies the zero
    model = """Layer is an entity type.
Stratum Stack is an entity type.
Engineering Lever is an entity type.
Load is a value type.
Layer stacks into Stratum Stack.
Layer is operator-loaded by Engineering Lever.
Layer has Load.

* Layer1 has Load '0' iff Layer1 stacks into Stratum Stack1 and Layer1 is operator-loaded by at most 0 Engineering Lever.

Layer 'L1' stacks into Stratum Stack 's'.
Layer 'L2' stacks into Stratum Stack 's'.
Layer 'L2' is operator-loaded by Engineering Lever 'e1'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(str(x) for x in r)
           for r in system._pop_rows(D, "Layer_has_Load")}
    # L1 idles at zero; L2 is loaded and gets NO zero row
    assert got == {("L1", "0")}


def test_deletions_propagate_through_derived_views():
    # the corpus's REAL shape (affect-select.md:38): ranks carries NO
    # uniqueness of its own, so the keyed pass cannot touch it — but Load IS
    # keyed, its zero-fill rows are superseded by counts, and rows ranks
    # derived from the superseded loads must RETRACT and rederive. This is
    # DRed (Gupta-Mumick-Subrahmanian 1993, in the library): delete the
    # overestimate through the dependency graph, rederive survivors.
    model = """Layer is an entity type.
Stratum Stack is an entity type.
Engineering Lever is an entity type.
Load is a value type.
Layer stacks into Stratum Stack.
Layer is operator-loaded by Engineering Lever.
Layer has Load.
Each Layer has at most one Load.
Stratum Stack ranks Layer at Load. *

* Layer1 has Load iff Load is the count of Engineering Lever1 where Layer1 is operator-loaded by Engineering Lever1.

* Layer1 has Load '0' iff Layer1 stacks into Stratum Stack1 and Layer1 is operator-loaded by at most 0 Engineering Lever.

* Stratum Stack1 ranks Layer1 at Load1 iff Layer1 stacks into Stratum Stack1 and Layer1 has Load1.

Layer 'L1' stacks into Stratum Stack 's'.
Layer 'L2' stacks into Stratum Stack 's'.
Layer 'L2' is operator-loaded by Engineering Lever 'e1'.
Layer 'L2' is operator-loaded by Engineering Lever 'e2'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    load = {tuple(str(x) for x in r)
            for r in system._pop_rows(D, "Layer_has_Load")}
    assert load == {("L1", "0"), ("L2", "2")}
    got = {tuple(str(x) for x in r)
           for r in system._pop_rows(D, "Stratum_Stack_ranks_Layer_at_Load")}
    # NO stale row for L2's superseded zero: the deletion propagated
    assert got == {("s", "L1", "0"), ("s", "L2", "2")}


def test_a_stale_derived_row_is_swept_without_a_fresh_supersession():
    # the frozen-store case (verdict ten, phase two): a fully-derived cell
    # carries a row inherited from an earlier compile — a per-invocation
    # trigger sees nothing superseded NOW and leaves it. For a fully-derived
    # head the stored cell is materialization of the expressible set (Codd
    # 1970 §1.5), never ground truth: run_rules must converge it whatever the
    # store's history, making derive idempotent.
    model = """Layer is an entity type.
Stratum Stack is an entity type.
Engineering Lever is an entity type.
Load is a value type.
Layer stacks into Stratum Stack.
Layer is operator-loaded by Engineering Lever.
Layer has Load.
Each Layer has at most one Load.
Stratum Stack ranks Layer at Load. *

* Layer1 has Load iff Load is the count of Engineering Lever1 where Layer1 is operator-loaded by Engineering Lever1.

* Layer1 has Load '0' iff Layer1 stacks into Stratum Stack1 and Layer1 is operator-loaded by at most 0 Engineering Lever.

* Stratum Stack1 ranks Layer1 at Load1 iff Layer1 stacks into Stratum Stack1 and Layer1 has Load1.

Layer 'L1' stacks into Stratum Stack 's'.
Layer 'L2' stacks into Stratum Stack 's'.
Layer 'L2' is operator-loaded by Engineering Lever 'e1'.
Layer 'L2' is operator-loaded by Engineering Lever 'e2'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    head = "Stratum_Stack_ranks_Layer_at_Load"
    rows = {tuple(r) for r in system._pop_rows(D, head)}
    stale = system._rowsort(rows | {("s", "L1", "9")})
    D = _store(head, stale, D)
    D = system.run_rules(D)
    got = {tuple(str(x) for x in r) for r in system._pop_rows(D, head)}
    assert got == {("s", "L1", "0"), ("s", "L2", "2")}


def test_aggregates_refold_after_downstream_sweeps():
    # the base-Depth regression (verdict ten): an aggregate folding OVER a
    # swept head ran before the sweep and kept the stale fold — the strata
    # must iterate to a JOINT fixpoint (loads settle, ranks rederives, the
    # peak refolds over the honest ranks)
    model = """Layer is an entity type.
Stratum Stack is an entity type.
Engineering Lever is an entity type.
Load is a value type.
Layer stacks into Stratum Stack.
Layer is operator-loaded by Engineering Lever.
Layer has Load.
Each Layer has at most one Load.
Stratum Stack ranks Layer at Load. *
Stratum Stack has peak Load. *
Each Stratum Stack has at most one peak Load.

* Layer1 has Load iff Load is the count of Engineering Lever1 where Layer1 is operator-loaded by Engineering Lever1.

* Layer1 has Load '0' iff Layer1 stacks into Stratum Stack1 and Layer1 is operator-loaded by at most 0 Engineering Lever.

* Stratum Stack1 ranks Layer1 at Load1 iff Layer1 stacks into Stratum Stack1 and Layer1 has Load1.

* Stratum Stack1 has peak Load iff Load is the max of Load1 where Stratum Stack1 ranks Layer1 at Load1.

Layer 'L1' stacks into Stratum Stack 's'.
Layer 'L2' stacks into Stratum Stack 's'.
Layer 'L2' is operator-loaded by Engineering Lever 'e1'.
Layer 'L2' is operator-loaded by Engineering Lever 'e2'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    ranks = "Stratum_Stack_ranks_Layer_at_Load"
    rows = {tuple(r) for r in system._pop_rows(D, ranks)}
    stale = system._rowsort(rows | {("s", "L9", "7")})
    D = _store(ranks, stale, D)
    D = system.run_rules(D)
    got = {tuple(str(x) for x in r)
           for r in system._pop_rows(D, "Stratum_Stack_has_peak_Load")}
    # the stale rank must not poison the refolded peak
    assert got == {("s", "2")}


def test_incremental_derive_propagates_through_all_three_passes():
    # the delta path: after an assert lands, run_rules(changed={ft}) must
    # carry the change through the count (agg), the keyed upsert, the ranks
    # rederive (sweep), and the peak refold (agg again) — the joint fixpoint
    # filtered by the dirty set, not a full re-evaluation of the store
    model = """Layer is an entity type.
Stratum Stack is an entity type.
Engineering Lever is an entity type.
Load is a value type.
Layer stacks into Stratum Stack.
Layer is operator-loaded by Engineering Lever.
Layer has Load.
Each Layer has at most one Load.
Stratum Stack ranks Layer at Load. *
Stratum Stack has peak Load. *
Each Stratum Stack has at most one peak Load.

* Layer1 has Load iff Load is the count of Engineering Lever1 where Layer1 is operator-loaded by Engineering Lever1.

* Layer1 has Load '0' iff Layer1 stacks into Stratum Stack1 and Layer1 is operator-loaded by at most 0 Engineering Lever.

* Stratum Stack1 ranks Layer1 at Load1 iff Layer1 stacks into Stratum Stack1 and Layer1 has Load1.

* Stratum Stack1 has peak Load iff Load is the max of Load1 where Stratum Stack1 ranks Layer1 at Load1.

Layer 'L1' stacks into Stratum Stack 's'.
Layer 'L2' stacks into Stratum Stack 's'.
Layer 'L2' is operator-loaded by Engineering Lever 'e1'.
Layer 'L2' is operator-loaded by Engineering Lever 'e2'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    ft = "Layer_is_operator_loaded_by_Engineering_Lever"
    rows = {tuple(r) for r in system._pop_rows(D, ft)}
    D = _store(ft, system._rowsort(rows | {("L1", "e3"), ("L1", "e4"),
                                           ("L1", "e5")}), D)
    D = system.run_rules(D, changed={ft})
    load = {tuple(str(x) for x in r)
            for r in system._pop_rows(D, "Layer_has_Load")}
    assert load == {("L1", "3"), ("L2", "2")}
    ranks = {tuple(str(x) for x in r)
             for r in system._pop_rows(D, "Stratum_Stack_ranks_Layer_at_Load")}
    assert ranks == {("s", "L1", "3"), ("s", "L2", "2")}
    peak = {tuple(str(x) for x in r)
            for r in system._pop_rows(D, "Stratum_Stack_has_peak_Load")}
    assert peak == {("s", "3")}


def test_keyed_heads_supersede_across_rounds():
    # verdict seven's cascade: a rule reading a cell that CHANGES across lfp
    # rounds (zero-fill lands round one, the count later) derives rows for
    # both values, and plain union keeps the stale one — ranks read 11 rows
    # where the old engine's keyed upsert (task-955) kept 8. A head whose
    # fact type carries a spanning uniqueness over its non-value roles
    # supersedes PER KEY from its rules' evaluation over the settled store.
    model = """Layer is an entity type.
Stratum Stack is an entity type.
Engineering Lever is an entity type.
Load is a value type.
Layer stacks into Stratum Stack.
Layer is operator-loaded by Engineering Lever.
Layer has Load.
Stratum Stack ranks Layer at Load.
In each population of Stratum Stack ranks Layer at Load, each Stratum Stack, Layer combination occurs at most once.

* Layer1 has Load iff Load is the count of Engineering Lever1 where Layer1 is operator-loaded by Engineering Lever1.

* Layer1 has Load '0' iff Layer1 stacks into Stratum Stack1 and Layer1 is operator-loaded by at most 0 Engineering Lever.

* Stratum Stack1 ranks Layer1 at Load1 iff Layer1 stacks into Stratum Stack1 and Layer1 has Load1.

Layer 'L1' stacks into Stratum Stack 's'.
Layer 'L2' stacks into Stratum Stack 's'.
Layer 'L2' is operator-loaded by Engineering Lever 'e1'.
Layer 'L2' is operator-loaded by Engineering Lever 'e2'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(str(x) for x in r)
           for r in system._pop_rows(D, "Stratum_Stack_ranks_Layer_at_Load")}
    # ONE row per (stack, layer): L1 at zero, L2 at its count — no stale rows
    assert got == {("s", "L1", "0"), ("s", "L2", "2")}


def test_count_and_zero_supply_coexist_on_one_head():
    # the corpus's full idiom (affect-select.md + the count-of-empty lesson):
    # the COUNT rule loads the loaded layers, the at-most-0 rule zero-fills the
    # idle ones — the aggregate stratum must supersede PER GROUP, never wipe the
    # whole head (the claude verdict: Load derived zero because agg-replace
    # clobbered the zero-supply rows)
    model = """Layer is an entity type.
Stratum Stack is an entity type.
Engineering Lever is an entity type.
Load is a value type.
Layer stacks into Stratum Stack.
Layer is operator-loaded by Engineering Lever.
Layer has Load.

* Layer1 has Load iff Load is the count of Engineering Lever1 where Layer1 is operator-loaded by Engineering Lever1.

* Layer1 has Load '0' iff Layer1 stacks into Stratum Stack1 and Layer1 is operator-loaded by at most 0 Engineering Lever.

Layer 'L1' stacks into Stratum Stack 's'.
Layer 'L2' stacks into Stratum Stack 's'.
Layer 'L2' is operator-loaded by Engineering Lever 'e1'.
Layer 'L2' is operator-loaded by Engineering Lever 'e2'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(str(x) for x in r)
           for r in system._pop_rows(D, "Layer_has_Load")}
    assert got == {("L1", "0"), ("L2", "2")}


def test_negation_over_an_absent_cell_is_vacuously_true():
    # the claude verdict's root cause: the neg side read a cell with NO rows
    # (nothing is operator-loaded anywhere) and the fetch bottomed instead of
    # answering the empty population — but 'at most 0 X' over nothing existing
    # must PASS everything
    model = """Layer is an entity type.
Stratum Stack is an entity type.
Engineering Lever is an entity type.
Load is a value type.
Layer stacks into Stratum Stack.
Layer is operator-loaded by Engineering Lever.
Layer has Load.

* Layer1 has Load '0' iff Layer1 stacks into Stratum Stack1 and Layer1 is operator-loaded by at most 0 Engineering Lever.

Layer 'L1' stacks into Stratum Stack 's'.
Layer 'L2' stacks into Stratum Stack 's'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(str(x) for x in r)
           for r in system._pop_rows(D, "Layer_has_Load")}
    assert got == {("L1", "0"), ("L2", "0")}


def test_the_negations_variable_shadows_the_outer_one():
    # 'no Transition' introduces a FRESH transition: the outer positive
    # Transition variable must not leak into the negated group
    model = """Node is an entity type.
Edge is an entity type.
Edge leaves Node.
Edge enters Node.
Node is a source.

* Node is a source iff some Edge leaves that Node and no Edge enters that Node.

Edge 'e1' leaves Node 'n1'.
Edge 'e1' enters Node 'n2'.
Edge 'e2' leaves Node 'n2'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(r) for r in system._pop_rows(D, "Node_is_a_source")}
    # n1: e1 leaves it, nothing enters it -> source. n2: e2 leaves it, but e1
    # enters it -> not a source (with leaked variables e1==e2 fails and n2
    # would wrongly qualify).
    assert got == {("n1",)}


def test_a_self_supporting_cycle_cannot_keep_itself_alive():
    # the recorded residual (GMS93, the paper's recursive case): a fully-
    # derived closure head is excluded from the whole-cell sweep because a
    # stale mutually-supporting pair rederives itself over a store that
    # still contains it. The paper's answer: delete the overestimate FIRST
    # (empty the cell), then rederive from remaining support to the local
    # fixpoint; rows with only cyclic support die, rows with base support
    # survive.
    model = """Node is an entity type.
Node links Node.
Node reaches Node. *

* Node1 reaches Node2 iff Node1 links Node2.

* Node1 reaches Node2 iff Node1 reaches Node3 and Node3 reaches Node2.

Node 'a' links Node 'b'.
Node 'b' links Node 'c'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    head = "Node_reaches_Node"
    honest = {tuple(r) for r in system._pop_rows(D, head)}
    assert honest == {("a", "b"), ("b", "c"), ("a", "c")}
    # history injects a mutually-supporting stale pair: x reaches y and y
    # reaches x, neither with any base support
    stale = system._rowsort(honest | {("x", "y"), ("y", "x"),
                                      ("x", "x"), ("y", "y")})
    D = _store(head, stale, D)
    D = system.run_rules(D)
    got = {tuple(r) for r in system._pop_rows(D, head)}
    assert got == honest


def test_a_vanished_groups_aggregate_row_dies_on_the_full_derive():
    # the residual's other half: per-group supersession keeps a group's fold
    # when the group's supply VANISHES from the store entirely (nothing
    # produces its key, so nothing supersedes it). For a FULLY-derived agg
    # head the cell is materialization, so the full derive replaces it whole
    # with the rules' fresh output (agg and paired plain rows together).
    model = """Layer is an entity type.
Engineering Lever is an entity type.
Load is a value type.
Layer is operator-loaded by Engineering Lever.
Layer has Load. *

* Layer1 has Load iff Load is the count of Engineering Lever1 where Layer1 is operator-loaded by Engineering Lever1.

Layer 'L1' is operator-loaded by Engineering Lever 'e1'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    head = "Layer_has_Load"
    honest = {tuple(r) for r in system._pop_rows(D, head)}
    assert honest == {("L1", 1)}
    # history: a fold for a layer whose levers vanished from the store
    D = _store(head, system._rowsort(honest | {("Lgone", 7)}), D)
    D = system.run_rules(D)
    got = {tuple(r) for r in system._pop_rows(D, head)}
    assert got == honest


def test_instance_membership_derives_from_the_store_itself():
    # proposal B (2026-07-04): the old engine MATERIALIZED
    # Resource_is_instance_of_Noun as a 12,539-row reflection cell; pyarest's
    # store IS that knowledge. The base's semantic-constraint and
    # machine-instance rules read the mirror, so the engine derives it: every
    # id playing one of a noun's roles is an instance of that noun, fresh
    # each derive, never migrated, never stale.
    model = """Status is a value type.
Resource is an entity type.
Noun is a value type.
Resource is instance of Noun.
Task is an entity type.
Task has Status.
Flag is a value type.
Task is flagged with Flag.
Resource is known.

* Resource1 is known iff Resource1 is instance of Noun 'Task'.

Task 't1' has Status 'open'.
Task 't2' is flagged with Flag 'red'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(r) for r in system._pop_rows(D, "Resource_is_instance_of_Noun")}
    # the ENGINE mirror knows both tasks (t2 plays only the flag role)
    assert ("t1", "Task") in got and ("t2", "Task") in got
    known = {r[0] for r in system._pop_rows(D, "Resource_is_known")}
    assert known == {"t1", "t2"}
