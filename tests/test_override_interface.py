"""The universal override interface (the layering discipline): a definition's canonical
meaning is its FFP object or lambda term; a host may register an optimized TWIN through
defs.override, resolution prefers the twin, and a host lacking one degrades gracefully
to the canonical form with the SAME result. The base natives are now Python's override
set registered through this interface, not a parallel store."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import defs, delta
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def test_the_base_natives_ride_the_interface():
    assert "apndl" in defs.fast and "COMP" in defs.fast       # the override set is explicit


def test_an_override_is_preferred_and_degrades_gracefully():
    ran = {"twin": 0}
    defs.define("twin_demo", S(A("COMP"), A(1), A("tl")))     # canonical: 2nd element

    def twin(mu, o):
        ran["twin"] += 1
        return o[1] if (type(o) is tuple and len(o) >= 2) else delta.BOT_D

    defs.override("twin_demo", twin)
    with_twin = from_lam(apply(A("twin_demo"), to_lam(("a", "b", "c"))))
    assert with_twin == "b" and ran["twin"] == 1              # the twin ran
    del defs.fast["twin_demo"]
    defs.version += 1
    without = from_lam(apply(A("twin_demo"), to_lam(("a", "b", "c"))))
    assert without == "b"                                     # graceful degradation: same result
