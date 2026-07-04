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
