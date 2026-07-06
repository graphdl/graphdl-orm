"""Literal bounds on a value role, CANONICALLY: a value constraint range ('The possible
values of Rating are at most 5.'), never an engine-invented dialect (no non-canonical
FORML). Enforcement legs: the value type's own cell flags offenders, and the routed
write through the RMAP layout refuses an offending column value (row_validate maps the
canonical constraint onto the column). NORMA's role-vs-role ValueComparisonConstraint
keeps its canonical verbalization for when that surface lands."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, defs, forml, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


MODEL = """Order(.OrderId) is an entity type.
Rating is a value type.
Order has Rating.
Each Order has at most one Rating.
The possible values of Rating are at most 5.
"""


def test_the_canonical_range_flags_offenders_on_the_value_cell():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    vo = forml.validate_for("Rating", D)
    with defs.step(D):
        (_p, V, flag) = from_lam(apply(vo, S(to_lam(((3,), (9,), (5,))), D)))
    assert set(V) == {(9,)}
    assert flag == "T"


def test_the_routed_write_refuses_an_offending_column_value():
    D, _ = forml.compile_model(MODEL)
    part = system.rmap_partition(D)
    assert part["Order_has_Rating"] == "Order"
    Dp = apply(A(2), system.create(D, "Order_has_Rating", to_lam(("o1", 9))))
    assert system.ft_view(Dp, "Order_has_Rating", part) == set()
    Dp2 = apply(A(2), system.create(D, "Order_has_Rating", to_lam(("o1", 3))))
    assert system.ft_view(Dp2, "Order_has_Rating", part) == {("o1", 3)}


def test_nf_is_idempotent_on_the_canonical_sentence():
    s = "The possible values of Rating are at most 5."
    assert forml.nf(s) == forml.nf(forml.nf(s))
