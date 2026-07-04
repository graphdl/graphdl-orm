"""The general conjunctive fragment: rule bodies whose clauses join on ANY bound
variables at ANY positions (Codd's join, §2.1.3, is not restricted to the running
tuple's last column; NORMA's role calculus and the ORM-to-datalog mapping compile
arbitrary conceptual joins). theta gains JoinOn — the multi-column equi-join built
from the SAME primitives as NatJoin (eq over selector tuples; no new host prims, so
every carrier runs it unchanged); compile_rule takes per-atom join specs and keeps
emitting NatJoin for the linear prefix it always compiled. A clause sharing NO bound
variable is the degenerate cross product (datalog-legal; the diagnostics narrow to
the genuinely unsafe cases: unbound head variables, no fact-type clause).

Executes the two recursive halves of aggregate_min_over_recursive_closure_e2e the
old engine could not run in-harness (its fix was verified on the deployed path
only), and the cross-antecedent value comparison of #914 (surface adapted to
numbered variables; the possessive navigation form is NORMA sugar over the same
bound-role comparison)."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import ast, forml, system
from pyarest import canon as theta
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_theta_joinon_joins_on_multiple_columns_and_keeps_the_fresh_ones():
    R = to_lam(((1, "x", 5), (1, "x", 7)))                    # ⟨r1..r3⟩ rows
    Sp = to_lam(((5, "u", "keep5"), (7, "v", "keep7"), (9, "w", "no")))
    out = from_lam(apply(theta.JoinOn(((3, 1),), (2, 3)), S(R, Sp)))
    assert set(out) == {(1, "x", 5, "u", "keep5"), (1, "x", 7, "v", "keep7")}
    # two-column join, keep nothing: a pure semijoin
    out2 = from_lam(apply(theta.JoinOn(((1, 1), (3, 2)), ()),
                          S(to_lam(((5, "a", "keep5"), (7, "b", "no"))),
                            to_lam(((5, "keep5"), (7, "other"))))))
    assert set(out2) == {(5, "a", "keep5")}


ARC = """Node(.Id) is an entity type.
Cost is a value type.
Node moves to Node at Cost.
Node reaches Node at Cost.
Node shortest reaches Node at Cost.
Cost plus Cost is Cost.
Node1 reaches Node2 at Cost1 if Node1 moves to Node2 at Cost1.
Node1 reaches Node2 at Cost3 if Node1 moves to Node3 at Cost1 and Node3 reaches Node2 at Cost2 and Cost1 plus Cost2 is Cost3.
Node1 shortest reaches Node2 at Cost3 if Node1 reaches Node2 at Cost2 and Cost3 is the min of Cost2.
"""


def _arc_D():
    D, rep = forml.compile_model(ARC)
    assert rep["unparsed"] == []
    assert rep["rule_diagnostics"] == []                      # the non-linear rule COMPILES
    D = apply(ast.Store("Node_moves_to_Node_at_Cost"),
              S(to_lam((("a", "b", 1), ("b", "c", 1), ("a", "c", 3))), D))
    D = apply(ast.Store("Cost_plus_Cost_is_Cost"), S(to_lam(((1, 1, 2),)), D))
    return system.run_rules(D)


def test_cost_summing_closure_derives_both_paths():
    Dpy = from_lam(_arc_D())
    # precondition (the old file's own): reaches(a,c) carries BOTH the direct 3 and
    # the a->b->c sum 1+1=2 — the mid-tuple join and the two-column plus join fired
    assert {(r[2]) for r in _cell(Dpy, "Node_reaches_Node_at_Cost")
            if r[0] == "a" and r[1] == "c"} == {2, 3}


def test_min_over_cost_summing_closure_folds_to_single_minimum():
    Dpy = from_lam(_arc_D())
    assert {r[2] for r in _cell(Dpy, "Node_shortest_reaches_Node_at_Cost")
            if r[0] == "a" and r[1] == "c"} == {2}


def test_single_path_groups_are_unaffected():
    Dpy = from_lam(_arc_D())
    shortest = _cell(Dpy, "Node_shortest_reaches_Node_at_Cost")
    assert {r[2] for r in shortest if r[:2] == ("a", "b")} == {1}
    assert {r[2] for r in shortest if r[:2] == ("b", "c")} == {1}


def test_cross_antecedent_comparison_filters_the_cartesian():
    MODEL = """Task(.id) is an entity type.
File(.path) is an entity type.
TaskId is a value type.
Task has TaskId.
Task is preceded.
Task touches File.
Task2 is preceded if Task1 has TaskId1 and Task2 has TaskId2 and TaskId1 is less than TaskId2 and Task1 touches File1 and Task2 touches File1.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    assert rep["rule_diagnostics"] == []
    D = apply(ast.Store("Task_has_TaskId"),
              S(to_lam((("1", "1"), ("2", "2"), ("3", "3"))), D))
    D = apply(ast.Store("Task_touches_File"),
              S(to_lam((("1", "f"), ("2", "f"), ("3", "f"))), D))
    D = system.run_rules(D)
    # three tasks share one file; the strict < cuts the cartesian to the directional
    # half: '2' (preceded by '1') and '3' (preceded by '1' or '2'), never '1'
    assert _cell(from_lam(D), "Task_is_preceded") == {("2",), ("3",)}
