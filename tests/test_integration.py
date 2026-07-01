import pyarest.reduce as R
from pyarest.defs import DEFS
from pyarest.objects import Atom, seq
import pyarest.primitives  # register primitives
import pyarest.forms as FR  # register forms


def setup_module(_):
    R.DEFS = DEFS


def test_compiled_definition_reduces_via_rho():
    # Def swap ≡ ρ⟨CONS,2,1⟩ ; then swap : ⟨a,b⟩ → ⟨b,a⟩
    DEFS.define("swap", seq(FR.CONS, Atom(2), Atom(1)))
    assert R.apply(Atom("swap"), seq(Atom("a"), Atom("b"))) == seq(Atom("b"), Atom("a"))


def test_boundary_holds_registered_not_compiled():
    keys = {d.key for d in DEFS.boundary()}
    assert "COMP" in keys and "tl" in keys and 1 in keys   # registered primitives/forms
    assert "swap" not in keys                              # compiled def is above the boundary
