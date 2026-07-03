"""A unary fact type maps to the entity directly (Halpin §10.3: the boolean column):
its internal uniqueness spans its single role BY DEFINITION — NORMA auto-creates that
UC on every unary role — so absorption needs no declaration. The routed write sets the
column to T (presence), ft_view reassembles the 1-tuples, and the # hole keeps the open
world (absence is unknown, not false, until a closed-world declaration says otherwise)."""
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


MODEL = """Task(.id) is an entity type.
Label is a value type.
Task is urgent.
Task is blocked.
Task has Label.
Each Task has at most one Label.
"""


def test_unaries_absorb_without_any_declaration():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    part = system.rmap_partition(D)
    assert part["Task_is_urgent"] == "Task"                   # inherent single-role UC
    assert part["Task_is_blocked"] == "Task"
    assert part["Task_has_Label"] == "Task"                   # the declared one, as before


def test_the_routed_unary_write_is_a_boolean_column():
    D, _ = forml.compile_model(MODEL)
    part = system.rmap_partition(D)
    D = apply(A(2), system.create(D, "Task_is_urgent", to_lam(("t1",))))
    D = apply(A(2), system.create(D, "Task_has_Label", to_lam(("t1", "red"))))
    Dpy = from_lam(D)
    row = next(c[2] for c in Dpy
               if isinstance(c, tuple) and len(c) == 3 and c[1] == "Task:t1")
    cols = system.table_columns(part, "Task")
    assert row[0] == "t1"
    assert row[1 + cols.index("Task_is_urgent")] == "T"       # presence, the boolean column
    assert row[1 + cols.index("Task_is_blocked")] == "#"      # open world: unknown
    assert row[1 + cols.index("Task_has_Label")] == "red"
    assert system.ft_view(D, "Task_is_urgent", part) == {("t1",)}
    assert system.ft_view(D, "Task_is_blocked", part) == set()
