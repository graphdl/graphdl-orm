"""Negation the NORMA way (docs/2026-07-02-negation-model.md): 'X is not R.' / 'X does
not R.' creates a PAIRED positive-shaped fact type linked by negOf, with the pair
exclusion auto-asserted (nothing is both) — negative information is stored as ordinary
monotone facts, so the substrate stays CALM. The closed world is the EXISTING
disjunctive-mandatory form over the pair ('Each Person smokes or does not smoke.'), and
defaults are read-time (#), never stored."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml
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
Person is an entity type.
Order is paid.
Order is not paid.
Person smokes.
Person does not smoke.
Each Person smokes or does not smoke.
"""


def test_negation_pairs_parse_into_M():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    Dpy = from_lam(D)
    negs = _cell(Dpy, "negOf")
    assert ("Order_is_not_paid", "Order_is_paid") in negs
    assert ("Person_does_not_smoke", "Person_smokes") in negs
    kinds = {(f[0], f[1]) for f in _cell(Dpy, "constraint") if len(f) >= 2}
    assert any(k == "exclusion" and c.startswith("negx_") for (c, k) in kinds)


def test_the_pair_exclusion_refuses_both():
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.Store("Order_is_paid"), S(to_lam((("o1",),)), D))
    vo = forml.validate_for("Order_is_not_paid", D)
    Dp = apply(A(2), ast.run(to_lam(("o1",)), D, validate_obj=vo, cell_name="Order_is_not_paid"))
    assert ("o1",) not in _cell(from_lam(Dp), "Order_is_not_paid")   # refused: o1 IS paid
    Dp2 = apply(A(2), ast.run(to_lam(("o2",)), D, validate_obj=vo, cell_name="Order_is_not_paid"))
    assert ("o2",) in _cell(from_lam(Dp2), "Order_is_not_paid")      # a different order: fine


def test_closed_world_is_the_existing_disjunctive_mandatory():
    D, _ = forml.compile_model(MODEL)
    rows = _cell(from_lam(D), "constraint")
    pair = ("Person_smokes", "Person_does_not_smoke")
    assert any(f[1] == "disjunctive_mandatory" and tuple(f[3]) == pair for f in rows if len(f) >= 4)


def test_nf_is_idempotent_on_the_negative_reading():
    assert forml.nf("Order is not paid.") == forml.nf(forml.nf("Order is not paid."))
