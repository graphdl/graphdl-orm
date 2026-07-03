"""The absorbed-population seam, closed: under RMAP absorption the fact type's own
population cell is maintained as a DERIVED-AND-STORED view (Halpin's ** marker applied
to the layout) in the SAME commit chain as the entity-cell write — so guards, rule
atoms, scoped constraints, and every other per-fact-type reader keep working unchanged,
the entity cell remains the write and isolation unit, and the view cache equals the
reassembly (ft_view) by construction. A refused step maintains neither."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, system
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


MODEL = """Task(.id) is an entity type.
Worker(.Name) is an entity type.
Task is ready.
Worker starts Task.
State Machine Definition 'Task' is for Noun 'Task'.
Status 'Todo' is initial in State Machine Definition 'Task'.
Transition 'start' is from Status 'Todo'.
Transition 'start' is to Status 'Doing'.
Transition 'start' is triggered by Fact Type 'Worker starts Task'.
Transition 'start' is guarded by Fact Type 'Task is ready'.
Task1 is startable if Task1 is ready.
"""


def test_the_view_cache_equals_the_reassembly():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    part = system.rmap_partition(D)
    assert part["Task_is_ready"] == "Task"                    # absorbed (inherent unary UC)
    D = apply(A(2), system.create(D, "Task_is_ready", to_lam(("t1",))))
    Dpy = from_lam(D)
    assert _cell(Dpy, "Task_is_ready") == {("t1",)}           # the ** cache, maintained
    assert system.ft_view(D, "Task_is_ready", part) == {("t1",)}   # equals the reassembly


def test_a_guard_on_an_absorbed_unary_reads_the_routed_write():
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.Store("Task_status"), S(to_lam((("t1", "Todo"),)), D))
    blocked = apply(A(2), system.create(D, "Worker_starts_Task", to_lam(("w1", "t1"))))
    assert _cell(from_lam(blocked), "Task_status") == {("t1", "Todo")}   # not ready yet
    D = apply(A(2), system.create(D, "Task_is_ready", to_lam(("t1",))))  # ROUTED write
    D = apply(A(2), system.create(D, "Worker_starts_Task", to_lam(("w1", "t1"))))
    assert _cell(from_lam(D), "Task_status") == {("t1", "Doing")}        # guard sees it


def test_a_rule_atom_over_an_absorbed_unary_derives():
    D, _ = forml.compile_model(MODEL)
    D = apply(A(2), system.create(D, "Task_is_ready", to_lam(("t7",))))
    D = system.run_rules(D)
    assert ("t7",) in _cell(from_lam(D), "Task_is_startable")
