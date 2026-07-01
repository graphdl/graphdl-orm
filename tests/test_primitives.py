import pyarest.reduce as R
from pyarest.defs import DEFS
from pyarest.objects import Atom, seq, PHI, T, F, BOTTOM
import pyarest.primitives  # registers on import


def setup_module(_):
    R.DEFS = DEFS   # use the shared store the primitives registered into


def test_selectors():
    s = seq(Atom("a"), Atom("b"), Atom("c"))
    assert R.apply(Atom(1), s) == Atom("a")
    assert R.apply(Atom(3), s) == Atom("c")
    assert R.apply(Atom(4), s) is BOTTOM     # out of range


def test_tl():
    assert R.apply(Atom("tl"), seq(Atom("a"), Atom("b"))) == seq(Atom("b"))
    assert R.apply(Atom("tl"), seq(Atom("a"))) == PHI


def test_id_atom_eq_null():
    assert R.apply(Atom("id"), Atom("z")) == Atom("z")
    assert R.apply(Atom("atom"), Atom(5)) == T
    assert R.apply(Atom("atom"), seq(Atom(1), Atom(2))) == F
    assert R.apply(Atom("eq"), seq(Atom(1), Atom(1))) == T
    assert R.apply(Atom("eq"), seq(Atom(1), Atom(2))) == F
    assert R.apply(Atom("null"), PHI) == T
    assert R.apply(Atom("null"), seq(Atom(1))) == F
