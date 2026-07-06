"""The clause-list scoped families from the shared source. participation is the
canonical builder over ⟨clauses, target⟩: each clause's rows tag as ⟨entity, clause⟩,
the target clause reading from P and the siblings from the frozen D, all flattened.
exclusion/exclusive_or/inclusive_or compose it by name; external uniqueness joins
the target with a named sibling on the shared key and applies the uniqueness builder
through the apply primitive. The pops override (the RMAP view seam) stays host-side,
as with the other scoped families."""
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


def _via(name, param, P, D):
    with defs.step(L.SEQ(L.NIL)):
        built = apply(A(name), param)
        return set(from_lam(apply(built, S(to_lam(P), D))))


def test_participation_tags_and_unions_target_and_siblings():
    D = _D(("ftB", (("e2", "x"),)))
    P = (("e1", "y"),)                                        # the target clause's cell
    got = _via("constraints:participation",
               S(to_lam(("ftA", "ftB")), A("ftA")), P, D)
    assert got == {("e1", "ftA"), ("e2", "ftB")}


def test_scoped_exclusion_from_the_canon():
    D = _D(("ftB", (("e1", "x"),)))                           # e1 also in the sibling
    P = (("e1", "y"), ("e2", "y"))
    got = _via("constraints:scoped_exclusion",
               S(to_lam(("ftA", "ftB")), A("ftA")), P, D)
    assert got == {("e1", "ftA"), ("e1", "ftB")}              # both participations
    assert set(from_lam(apply(C.scoped_exclusion(("ftA", "ftB"), "ftA"),
                              S(to_lam(P), D)))) == got


def test_scoped_inclusive_or_from_the_canon():
    D = _D(("Person", (("p1",), ("p2",), ("p3",))), ("ftB", (("p2", "x"),)))
    P = (("p1", "y"),)
    got = _via("constraints:scoped_inclusive_or",
               S(A("Person"), to_lam(("ftA", "ftB")), A("ftA")), P, D)
    assert got == {("p3",)}                                   # in NO clause
    assert set(from_lam(apply(C.scoped_inclusive_or("Person", ("ftA", "ftB"), "ftA"),
                              S(to_lam(P), D)))) == got


def test_scoped_exclusive_or_from_the_canon():
    D = _D(("Person", (("p1",), ("p2",), ("p3",))), ("ftB", (("p1", "x"),)))
    P = (("p1", "y"),)                                        # p1 in BOTH clauses
    got = _via("constraints:scoped_exclusive_or",
               S(A("Person"), to_lam(("ftA", "ftB")), A("ftA")), P, D)
    assert got == {("p2",), ("p3",), ("p1",)}                 # none-holders + many-holder
    assert set(from_lam(apply(C.scoped_exclusive_or("Person", ("ftA", "ftB"), "ftA"),
                              S(to_lam(P), D)))) == got


def test_scoped_external_uniqueness_from_the_canon():
    # Halpin 10.21: UC spanning cols of the natural join of two tables. Join P with
    # the sibling on role 1; the joined tuples sharing cols 2..3 with a DIFFERENT
    # tuple violate.
    D = _D(("other", (("a", "k1"), ("b", "k1"), ("c", "k2"))))
    P = (("a", "v"), ("b", "v"), ("c", "v"))
    got = _via("constraints:scoped_external_uniqueness",
               S(A("other"), to_lam((2, 3))), P, D)
    assert got == {("a", "v", "k1"), ("b", "v", "k1")}
    assert set(from_lam(apply(C.scoped_external_uniqueness("other", [2, 3]),
                              S(to_lam(P), D)))) == got
