"""The remaining Backus base (spec §4.2 / §11.2.3–11.2.4): trans, rotl, rotr, div, and
the bu (binary-to-unary) form — on both paths, with ⊥ outside the stated shapes."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import reduce as R


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def both(f, x):
    xl = to_lam(x)
    lam_r, dlt_r = from_lam(R.apply_lambda(f, xl)), from_lam(R.apply(f, xl))
    assert lam_r == dlt_r, (lam_r, dlt_r)
    return lam_r


def test_trans_transposes():
    assert both(A("trans"), (("a", "b"), ("c", "d"), ("e", "f"))) == (("a", "c", "e"), ("b", "d", "f"))
    assert both(A("trans"), ()) == ()


def test_trans_is_bottom_on_ragged_or_atom_rows():
    assert both(A("trans"), (("a", "b"), ("c",))) == "⊥"        # ragged
    assert both(A("trans"), (("a",), "b")) == "⊥"               # an atom row
    assert both(A("trans"), "x") == "⊥"                          # not a sequence


def test_rotl_rotr():
    assert both(A("rotl"), ("a", "b", "c")) == ("b", "c", "a")
    assert both(A("rotr"), ("a", "b", "c")) == ("c", "a", "b")
    assert both(A("rotl"), ()) == ()
    assert both(A("rotr"), ()) == ()
    assert both(A("rotl"), "x") == "⊥"


def test_div():
    assert both(A("div"), (6, 3)) == 2.0
    assert both(A("div"), (1, 0)) == "⊥"                         # ÷0 = ⊥
    assert both(A("div"), ("a", 2)) == "⊥"


def test_bu_binary_to_unary():
    # (bu + 1) : 5  =  +:⟨1,5⟩  =  6   (Backus §11.2.4)
    inc = S(A("BU"), A("+"), A(1))
    assert both(inc, 5) == 6
    # (bu cat ⟨a⟩) : ⟨b⟩ = ⟨a,b⟩ — the bound arg is quoted data
    pre = S(A("BU"), A("cat"), S(A("a")))
    assert both(pre, ("b",)) == ("a", "b")
