from pyarest.objects import Atom, Seq, seq, BOTTOM
from pyarest.defs import Defs
import pyarest.reduce as R


def test_registered_primitive_is_applied():
    d = Defs()
    d.register("first", lambda x: x.items[0])
    R.DEFS = d   # inject a clean store for the test
    assert R.apply(Atom("first"), seq(Atom("a"), Atom("b"))) == Atom("a")


def test_undefined_atom_denotes_bottom_function():
    R.DEFS = Defs()
    assert R.apply(Atom("undefined"), Atom("x")) is BOTTOM


def test_metacomposition_routes_to_head_controlling_operator():
    # ρ⟨HEAD, extra⟩ : y  ==  ρHEAD : ⟨⟨HEAD, extra⟩, y⟩
    d = Defs()
    seen = {}

    def head(arg):        # arg should be ⟨⟨HEAD, extra⟩, y⟩
        seen["arg"] = arg
        return Atom("ok")

    d.register("HEAD", head)
    R.DEFS = d
    whole = seq(Atom("HEAD"), Atom("extra"))
    assert R.apply(whole, Atom("y")) == Atom("ok")
    assert seen["arg"] == seq(whole, Atom("y"))
