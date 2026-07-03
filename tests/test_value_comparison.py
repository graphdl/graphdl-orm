"""NORMA's value-comparison constraint family (the paper's Def. Schema lists it; the
corpus's Word Comparator vocabulary carries the surfaces): '<reading> is at most 5.'
constrains the reading's value role against the literal; offenders are the violations
and an alethic offender never lands. Role-vs-role comparison across fact types is the
recorded next surface."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, defs, forml
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


MODEL = """Order(.OrderId) is an entity type.
Rating is a value type.
Order has Rating.
Order has Rating is at most 5.
"""


def test_value_comparison_flags_the_offending_rows():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    vo = forml.validate_for("Order_has_Rating", D)
    pop = to_lam((("o1", 3), ("o2", 9), ("o3", 5)))
    with defs.step(D):
        (_p, V, flag) = from_lam(apply(vo, S(pop, D)))
    assert set(V) == {("o2", 9)}                              # only the offender
    assert flag == "T"                                        # alethic


def test_an_offending_write_never_lands():
    D, _ = forml.compile_model(MODEL)
    vo = forml.validate_for("Order_has_Rating", D)
    Dp = apply(A(2), ast.run(to_lam(("o2", 9)), D, validate_obj=vo, cell_name="Order_has_Rating"))
    assert ("o2", 9) not in _cell(from_lam(Dp), "Order_has_Rating")
    Dp2 = apply(A(2), ast.run(to_lam(("o1", 3)), D, validate_obj=vo, cell_name="Order_has_Rating"))
    assert ("o1", 3) in _cell(from_lam(Dp2), "Order_has_Rating")


def test_nf_is_idempotent_on_the_comparison():
    assert forml.nf("Order has Rating is at most 5.") == \
        forml.nf(forml.nf("Order has Rating is at most 5."))
