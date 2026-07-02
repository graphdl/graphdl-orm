"""Semi-naive evaluation (Bancilhon–Ramakrishnan 1986, in the library): after round one,
run_rules joins each rule only against the PER-ROUND DELTA of what changed, through
stored ~d variants of the rule body (one per atom position, the atom reading ⟨Δ, D⟩'s
first element instead of its cell). Sound and complete because every genuinely new tuple
uses at least one new row. The observable: by the later rounds the delta input is
strictly smaller than the full population the naive loop would have re-joined; the
result is the same lfp."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


MODEL = """Person is an entity type.
Person is a parent of Person.
Person1 is an ancestor of Person2 if Person1 is a parent of Person2.
Person1 is an ancestor of Person3 if Person1 is a parent of Person2 and Person2 is an ancestor of Person3.
"""

CHAIN = (("a", "b"), ("b", "c"), ("c", "d"), ("d", "e"))
CLOSURE = {("a", "b"), ("a", "c"), ("a", "d"), ("a", "e"), ("b", "c"), ("b", "d"),
           ("b", "e"), ("c", "d"), ("c", "e"), ("d", "e")}


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_semi_naive_reaches_the_same_lfp_with_delta_sized_joins():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    D = apply(ast.Store("Person_is_a_parent_of_Person"), S(to_lam(CHAIN), D))
    stats = []
    D = system.run_rules(D, stats=stats)
    assert _cell(from_lam(D), "Person_is_an_ancestor_of_Person") == CLOSURE
    deltas = [r for r in stats if r["mode"] == "delta"]
    assert deltas, "rounds after the first must join deltas, not full populations"
    assert any(r["in"] < r["base"] for r in deltas)           # strictly smaller join input
    assert all(r["mode"] == "full" for r in stats if r["round"] == 1)


def test_the_frontier_still_gates_round_one():
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.Store("Person_is_a_parent_of_Person"), S(to_lam(CHAIN), D))
    stats = []
    system.run_rules(D, changed={"unrelated_cell"}, stats=stats)
    assert stats == []                                        # nothing read, nothing fired
