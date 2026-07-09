"""Trailing x-if-y subset constraints (the set-comparison arc, slice 2):
FORML 2's implication clause on an ASSERTED head checks — pi_bound(cond)
must be included in pi_bound(head), the roles bound by the condition's
that-anaphors. Halpin, Mapping ORM to Datalog: '<-' is read as 'if';
an asserted head has no rule to close, so the implication is the '->'
constraint direction."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system
from pyarest.lam import to_lam, from_lam
from pyarest.reduce import apply as _apply
import pyarest.lam as L

MODEL = """
Agent(.System Name) is an entity type.
Human(.Person Name) is an entity type.
Order(.Order Id) is an entity type.
System Name is a value type.
Person Name is a value type.
Order Id is a value type.

Agent obeys Order.
Human issues Order.

Agent obeys Order if Human issues that Order.
"""


def _compile():
    D, rep = forml.compile_model(MODEL)
    assert not rep.get("unclassified"), rep
    return D


def _violations(D, ft, pop):
    val = forml.validate_for(ft, D, system.rmap_partition(D))
    pair = L.SEQ(L.CONS(to_lam(tuple(tuple(r) for r in pop)))(
        L.CONS(D)(L.NIL)))
    from pyarest import defs
    with defs.step(D):
        _p, v, _f = from_lam(_apply(val, pair))
    return list(v)


def test_subset_constraint_minted():
    D = _compile()
    rows = [tuple(r) for r in system._pop_rows(D, "constraint")]
    subs = [r for r in rows if len(r) >= 2 and r[1] == "subset"]
    assert subs, rows


def test_unmatched_issue_violates():
    D = _compile()
    v = _violations(D, "Human_issues_Order", [("h1", "o1")])
    assert v, "an issued Order nobody obeys must violate the subset"


def test_matched_issue_is_clean():
    D = _compile()
    D = _apply(
        L.atom(2),
        __import__("pyarest").ast.run(
            to_lam(("a1", "o1")), D, cell_name="Agent_obeys_Order"))
    v = _violations(D, "Human_issues_Order", [("h1", "o1")])
    assert not v, v


def test_projection_ignores_unbound_roles():
    D = _compile()
    D = _apply(
        L.atom(2),
        __import__("pyarest").ast.run(
            to_lam(("a9", "o2")), D, cell_name="Agent_obeys_Order"))
    # a DIFFERENT human issuing o2 satisfies the Order-projected subset:
    # the Human and Agent roles are unbound and project away
    v = _violations(D, "Human_issues_Order", [("h2", "o2")])
    assert not v, v
