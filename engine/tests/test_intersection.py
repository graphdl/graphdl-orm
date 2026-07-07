"""INTERSECTION SOURCE: shared/theta.canon is one file, consumed verbatim by both
hosts. Python execs it under the vocabulary binding (canon.load); the Rust kernel
include!s the identical bytes into a function defining the same vocabulary over its
V, and resolves the names at reduction like any compiled definition. The tests hold
the file to its meaning: every canonical definition loaded from the shared file
reduces exactly like python/theta.canon's constructed object (the authoring toolchain),
on both kernels. No JSON shim, no parser, no tree format: the source is the source."""
import pytest
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import canon, constraints as C, defs, polyglot
from pyarest import canon as theta
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

# the higher-order builders: canonical NAME applied to the PARAMETER yields the
# object, which applied to the operand must reduce like the toolchain constructor
FPOP = (("a", 1), ("b", 2), ("c", 1))
OTHER = (("a", "x"), ("c", "y"))
HIGHER = [
    ("theta:Filter", A("eq"), theta.Filter(A("eq")), None),
    ("theta:NatJoin", A(2), theta.NatJoin(2), None),
    ("theta:Project", to_lam((2,)), theta.Project([2]), None),
    ("constraints:uniqueness", to_lam((1,)), C.uniqueness([1]), None),
]
HIGHER_X = {
    "theta:Filter": to_lam((("x", "x"), ("x", "y"), ("z", "z"))),
    "theta:NatJoin": S(to_lam(FPOP), to_lam(OTHER)),
    "theta:Project": to_lam(FPOP),
    "constraints:uniqueness": to_lam(POP),
}
CLOSED_C = [
    ("constraints:mandatory", C.mandatory(), S(to_lam(("e1", "e2", "e3")), to_lam(("e1",)))),
    ("constraints:subset", C.subset(), S(to_lam(POP), to_lam((("a", 1),)))),
    ("constraints:equality", C.equality(), S(to_lam((("a",), ("b",))), to_lam((("b",), ("c",))))),
    ("constraints:exclusion", C.exclusion(), to_lam((("e1", "ft1"), ("e1", "ft2"), ("e2", "ft1")))),
]


def _loaded_D():
    canon.load_all()
    return L.SEQ(L.NIL)


def test_higher_order_builders_reduce_like_the_constructors():
    D = _loaded_D()
    for name, param, built, _ in HIGHER:
        x = HIGHER_X[name]
        with defs.step(D):
            obj = apply(A(name), param)                       # the canonical builder
            got = from_lam(apply(obj, x))                     # applied to the operand
            want = from_lam(apply(built, x))
        assert got == want, name


def test_closed_constraint_families_reduce_like_the_constructors():
    D = _loaded_D()
    for name, built, x in CLOSED_C:
        with defs.step(D):
            got = from_lam(apply(A(name), x))
            want = from_lam(apply(built, x))
        assert got == want, name


@pytest.mark.skipif(not polyglot.rust_available(),
                    reason="rust kernel not built (cd rust; cargo build --release)")
def test_both_kernels_build_and_reduce_the_higher_order_canon():
    D = _loaded_D()
    cases = []
    for name, param, _b, _ in HIGHER:
        # ⟨COMP, apply, ⟨CONS, ⟨COMP, name, K(param)⟩, id⟩⟩ : x — build THEN apply,
        # one case, entirely through each kernel's own resolution
        f = S(A("COMP"), A("apply"),
              S(A("CONS"), S(A("COMP"), A(name), S(A("CONST"), param)), A("id")))
        cases.append((f, HIGHER_X[name], None))
    for name, _b, x in CLOSED_C:
        cases.append((A(name), x, None))
    got = polyglot.run_rust(polyglot.export_scenario(D, cases))
    want = polyglot.python_ground_truth(D, cases)
    names = [h[0] for h in HIGHER] + [c[0] for c in CLOSED_C]
    for (name, g, w) in zip(names, got, want):
        assert g == w, f"intersection divergence on {name}: rust={g!r} python={w!r}"


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
