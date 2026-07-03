"""INTERSECTION SOURCE: shared/theta.py is one file, consumed verbatim by both
hosts. Python execs it under the vocabulary binding (canon.load); the Rust kernel
include!s the identical bytes into a function defining the same vocabulary over its
V, and resolves the names at reduction like any compiled definition. The tests hold
the file to its meaning: every canonical definition loaded from the shared file
reduces exactly like python/theta.py's constructed object (the authoring toolchain),
on both kernels. No JSON shim, no parser, no tree format: the source is the source."""
import pytest
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import canon, defs, polyglot, theta
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


POP = (("a", 1), ("b", 2), ("a", 1))
CASES = [
    ("theta:member", theta.member, S(to_lam(("a", 1)), to_lam(POP))),
    ("theta:dedup", theta.dedup, to_lam(POP)),
    ("theta:flatten", theta.flatten, to_lam((("a", 1), ("b",), ()))),
    ("theta:setminus", theta.setminus, S(to_lam(POP), to_lam((("a", 1),)))),
    ("theta:Tie", theta.Tie, to_lam((("a", 1, "a"), ("b", 2, "c")))),
]


def _loaded_D():
    canon.load("theta.py")
    return L.SEQ(L.NIL)


def test_the_shared_file_reduces_like_the_toolchain_constructors():
    D = _loaded_D()
    for name, built, x in CASES:
        with defs.step(D):
            got = from_lam(apply(A(name), x))                 # by NAME, through rho
            want = from_lam(apply(built, x))
        assert got == want, name


@pytest.mark.skipif(not polyglot.rust_available(),
                    reason="rust kernel not built (cd rust; cargo build --release)")
def test_both_kernels_consume_the_identical_file():
    D = _loaded_D()
    cases = [(A(name), x, None) for name, _b, x in CASES]
    got = polyglot.run_rust(polyglot.export_scenario(D, cases))
    want = polyglot.python_ground_truth(D, cases)
    for ((name, _b, _x), g, w) in zip(CASES, got, want):
        assert g == w, f"intersection divergence on {name}: rust={g!r} python={w!r}"
