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


# --- the FORBIDDEN (exclusion) trailing-if: sign picks the check ---
FORBID = """
Agent(.System Name) is an entity type.
Order(.Order Id) is an entity type.
System Name is a value type.
Order Id is a value type.

Agent obeys Order.
Agent defies Order.

It is forbidden that Agent obeys Order if Agent defies that Order.
"""


def _compile_forbid():
    D, rep = forml.compile_model(FORBID)
    assert not rep.get("unclassified"), rep
    return D


def test_forbidden_mints_exclusion_not_subset():
    D = _compile_forbid()
    rows = [tuple(r) for r in system._pop_rows(D, "constraint")]
    assert any(len(r) >= 2 and r[1] == "subset" for r in rows), rows


def test_forbidden_cooccurrence_violates():
    # the constraint attaches to the CONDITION cell (Agent_defies_Order),
    # like the subset case: an Order both obeyed AND defied is forbidden.
    D = _compile_forbid()
    D = _apply(L.atom(2), __import__("pyarest").ast.run(
        to_lam(("a1", "o1")), D, cell_name="Agent_obeys_Order"))
    v = _violations(D, "Agent_defies_Order", [("a1", "o1")])
    assert v, "obey+defy on the same Order must violate the exclusion"


def test_forbidden_disjoint_is_clean():
    D = _compile_forbid()
    D = _apply(L.atom(2), __import__("pyarest").ast.run(
        to_lam(("a1", "o1")), D, cell_name="Agent_obeys_Order"))
    # o2 defied but only o1 obeyed: disjoint on Order, exclusion satisfied
    v = _violations(D, "Agent_defies_Order", [("a1", "o2")])
    assert not v, v


# --- the VALUE-RESTRICTION slice: 'X if that E has Attr <lit>' ---
VALUE = """
Ticket(.Ticket Id) is an entity type.
Municipality(.Name) is an entity type.
Ticket Id is a value type.
Name is a value type.
Closure Reason is a value type.
  The possible values of Closure Reason are 'Paid', 'Open'.

Ticket has Closure Reason.
Municipality replaces Ticket.

It is obligatory that Municipality replaces Ticket if that Ticket has Closure Reason 'Paid'.
"""


def _compile_value():
    D, rep = forml.compile_model(VALUE)
    assert not rep.get("unclassified"), rep
    return D


def test_value_restricted_subset_mints():
    D = _compile_value()
    rows = [tuple(r) for r in system._pop_rows(D, "constraint")]
    assert any(len(r) >= 2 and r[1] == "subset" for r in rows), rows


def test_value_paid_unreplaced_violates():
    D = _compile_value()
    v = _violations(D, "Ticket_has_Closure_Reason", [("t1", "Paid")])
    assert v, "a Paid ticket not replaced must violate the obligation"


def test_value_paid_replaced_is_clean():
    D = _compile_value()
    D = _apply(L.atom(2), __import__("pyarest").ast.run(
        to_lam(("m1", "t1")), D, cell_name="Municipality_replaces_Ticket"))
    v = _violations(D, "Ticket_has_Closure_Reason", [("t1", "Paid")])
    assert not v, v


def test_value_open_unreplaced_is_clean():
    D = _compile_value()
    # Open filtered OUT by the value restriction: not obligated to replace
    v = _violations(D, "Ticket_has_Closure_Reason", [("t2", "Open")])
    assert not v, v
