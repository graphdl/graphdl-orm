"""Bounded recomputation, wired (Cor. streaming): run_rules accepts the set of changed
cells and re-fires ONLY the rules whose ruleReads intersect it, then feeds what those
rules derive into the next round (meta's frontier). Observable behavior: a stale head is
NOT repaired by a change to an unrelated cell, and IS repaired by a change to a read cell."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


MODEL = """Person is an entity type.
Person is a parent of Person.
Person1 is a grandparent of Person2 if Person1 is a parent of some Person3 and Person3 is a parent of Person2.
"""


def _with_pop(D, name, pop):
    return apply(ast.Store(name), L.SEQ(L.CONS(to_lam(pop))(L.CONS(D)(L.NIL))))


def _cell(D, name):
    for c in from_lam(D):
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_unrelated_change_does_not_refire_the_rule():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Person_is_a_parent_of_Person", (("a", "b"), ("b", "c")))
    D = _with_pop(D, "Unrelated", (("x",),))
    D = system.run_rules(D, changed=["Unrelated"])
    assert _cell(D, "Person_is_a_grandparent_of_Person") == set()   # stale head stays stale


def test_read_cell_change_refires_and_cascades():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Person_is_a_parent_of_Person", (("a", "b"), ("b", "c")))
    D = system.run_rules(D, changed=["Person_is_a_parent_of_Person"])
    assert _cell(D, "Person_is_a_grandparent_of_Person") == {("a", "c")}


def test_no_changed_argument_means_run_everything():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Person_is_a_parent_of_Person", (("a", "b"), ("b", "c")))
    D = system.run_rules(D)
    assert _cell(D, "Person_is_a_grandparent_of_Person") == {("a", "c")}
