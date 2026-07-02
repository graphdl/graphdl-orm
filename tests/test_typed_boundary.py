"""Def. reg's dom/cod as typed-predicate constraints (DatalogLB right-arrow types, per
the writer-model source pass): a registered function's signature is M facts (defSig
rows ⟨name, position, objectType⟩, cod at position 0), and checked_apply admits the
application iff every argument at a declared dom position is an instance of its object
type (membership in the type's index cell), answering ERROR otherwise — the transition
rule refuses it. An undeclared name applies unchecked; cod's enforcement is the
receiving cell's own constraints on the next write."""
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


def _promote_impl(mu):
    def g(o):
        it = defs._items(L._list(o))
        v = defs._aval(it[0]) if it else None
        return L.atom(f"VIP:{v}") if v is not None else L.BOT
    return g


MODEL = "Order(.OrderId) is an entity type.\n"


def test_the_typed_boundary_admits_typed_args_and_refuses_others():
    defs.register("promote", _promote_impl)
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.Store("Order"), S(to_lam((("o1",),)), D))   # the noun's index cell
    D = system.declare_sig(D, "promote", ("Order",), "Badge")
    ca = system.checked_apply("promote")
    assert from_lam(apply(ca, S(to_lam(("o1",)), D))) == "VIP:o1"
    assert from_lam(apply(ca, S(to_lam(("zz",)), D))) == "ERROR"   # zz is no Order


def test_an_undeclared_sig_applies_unchecked():
    defs.register("promote", _promote_impl)
    D, _ = forml.compile_model(MODEL)
    ca = system.checked_apply("promote")
    assert from_lam(apply(ca, S(to_lam(("anything",)), D))) == "VIP:anything"
