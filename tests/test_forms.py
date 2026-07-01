import pyarest.reduce as R
from pyarest.defs import DEFS
from pyarest.objects import Atom, seq
import pyarest.primitives   # selectors/tl needed
import pyarest.forms as FR


def setup_module(_):
    R.DEFS = DEFS


def test_const_ignores_operand():
    # ⟨CONST, a⟩ : b  →  a
    assert R.apply(seq(FR.CONST, Atom("a")), Atom("b")) == Atom("a")


def test_cons_is_construction():
    # ⟨CONS, 2, 1⟩ : ⟨a, b⟩  →  ⟨2:⟨a,b⟩, 1:⟨a,b⟩⟩ = ⟨b, a⟩   (swap)
    swap = seq(FR.CONS, Atom(2), Atom(1))
    assert R.apply(swap, seq(Atom("a"), Atom("b"))) == seq(Atom("b"), Atom("a"))


def test_comp_is_composition():
    # ⟨COMP, 1, tl⟩ : ⟨a, b, c⟩  →  1:(tl:⟨a,b,c⟩) = 1:⟨b,c⟩ = b
    f = seq(FR.COMP, Atom(1), Atom("tl"))
    assert R.apply(f, seq(Atom("a"), Atom("b"), Atom("c"))) == Atom("b")
