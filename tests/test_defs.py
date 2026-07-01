from pyarest.objects import Atom
from pyarest.defs import Defs


def test_register_then_lookup_returns_callable_impl():
    d = Defs()
    d.register("id", lambda x: x)
    got = d.get("id")
    assert got.origin == "registered"
    assert got.impl(Atom(3)) == Atom(3)


def test_define_stores_a_compiled_object():
    d = Defs()
    d.define("k", Atom("body"))
    got = d.get("k")
    assert got.origin == "compiled"
    assert got.impl == Atom("body")


def test_boundary_is_registered_defs_only():
    d = Defs()
    d.register("r", lambda x: x)
    d.define("c", Atom("x"))
    assert {b.key for b in d.boundary()} == {"r"}


def test_missing_key_returns_none():
    assert Defs().get("nope") is None
