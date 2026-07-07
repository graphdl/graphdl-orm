"""The string-parameterized scoped constraint families from the shared source. A
scoped violation expression consumes ⟨P, D⟩, fetching sibling populations from the
frozen D through ast:FetchPop — which is why these waited for the ast wave. The
strict gates hand-build stores with sibling cells and assert absolute violations per
family; the wrapper must agree with the canonical name. The EXPRESSION-parameterized
branch (the RMAP view seam, where the sibling is an absorbed fact type reassembled
through the index) stays host-side until system.canon migrates, and the wrappers keep
that branch."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import constraints as C, defs
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _D(*cells):
    return to_lam(tuple(("CELL", n, v) for (n, v) in cells))


def _run(obj, P, D):
    with defs.step(L.SEQ(L.NIL)):
        return set(from_lam(apply(obj, S(to_lam(P), D))))


def _name(name, cell, P, D):
    with defs.step(L.SEQ(L.NIL)):
        built = apply(A(name), A(cell))
        return set(from_lam(apply(built, S(to_lam(P), D))))


def test_scoped_subset_from_the_canon():
    D = _D(("B", (("a",),)))
    P = (("a",), ("c",))
    assert _name("constraints:scoped_subset", "B", P, D) == {("c",)}
    assert _run(C.scoped_subset("B"), P, D) == {("c",)}
    # absent sibling: everything in P violates (pop_of defaults to the empty pop)
    assert _name("constraints:scoped_subset", "ZZ", P, D) == {("a",), ("c",)}


def test_scoped_mandatory_both_attachments_from_the_canon():
    D = _D(("Person", (("p1",), ("p2",))), ("F", (("p2", "y"),)))
    facts = (("p1", "x"),)
    assert _name("constraints:scoped_mandatory_entities", "Person", facts, D) \
        == {("p2",)}
    assert _run(C.scoped_mandatory_entities("Person"), facts, D) == {("p2",)}
    entities = (("p1",), ("p2",))
    assert _name("constraints:scoped_mandatory_facts", "F", entities, D) == {("p1",)}
    assert _run(C.scoped_mandatory_facts("F"), entities, D) == {("p1",)}


def test_scoped_equality_side_from_the_canon():
    D = _D(("O", (("b",), ("c",))))
    P = (("a",), ("b",))
    assert _name("constraints:scoped_equality_side", "O", P, D) == {("a",), ("c",)}
    assert _run(C.scoped_equality_side("O"), P, D) == {("a",), ("c",)}


def test_the_expression_branch_stays_host_side_and_working():
    # the RMAP view seam: a ready population EXPRESSION over D instead of a name
    D = _D(("B", (("a",),)))
    expr = __import__("pyarest.ast", fromlist=["FetchPop"]).FetchPop("B")
    v = _run(C.scoped_subset(expr), (("a",), ("c",)), D)
    assert v == {("c",)}
