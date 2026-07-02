"""The differential oracle (audit A4): the λ kernel and the δ fast-path are ONE semantics.
A seeded metamorphic sweep — randomly generated FFP objects applied to randomly generated
operands, malformed shapes and ⊥-producing paths included — asserts pointwise agreement.
This is the contract that licenses keeping a native evaluator at all (spec D5)."""
import random
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import reduce as R
from pyarest import theta as T, constraints as C


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def agree(f, x, tag):
    xl = to_lam(x)
    lam_r = from_lam(R.apply_lambda(f, xl))
    dlt_r = from_lam(R.apply(f, xl))
    assert lam_r == dlt_r, (tag, x, lam_r, dlt_r)


_LEAF_ATOMS = ["a", "b", 1, 2, "T", "F"]


def gen_obj(rng, depth):
    if depth == 0 or rng.random() < 0.45:
        return rng.choice(_LEAF_ATOMS)
    return tuple(gen_obj(rng, depth - 1) for _ in range(rng.randint(0, 3)))


def _leaves(rng):
    return [A(rng.randint(1, 4)), A("tl"), A("tlr"), A("1r"), A("id"), A("atom"),
            A("null"), A("eq"), A("reverse"), A("length"), A("cat"), A("apndl"),
            A("apndr"), A("distl"), A("distr"), A("not"), A("and"), A("or"),
            A("trans"), A("rotl"), A("rotr"), A("+"), A("div"), A("ge"),
            S(A("CONST"), to_lam(gen_obj(rng, 1)))]


def gen_fn(rng, depth):
    if depth == 0:
        return rng.choice(_leaves(rng))
    k = rng.random()
    if k < 0.25:
        return S(A("COMP"), gen_fn(rng, depth - 1), gen_fn(rng, depth - 1))
    if k < 0.45:
        n = rng.randint(1, 3)
        return S(A("CONS"), *[gen_fn(rng, depth - 1) for _ in range(n)])
    if k < 0.58:
        return S(A("ALPHA"), gen_fn(rng, depth - 1))
    if k < 0.72:
        return S(A("COND"), gen_fn(rng, depth - 1), gen_fn(rng, depth - 1), gen_fn(rng, depth - 1))
    if k < 0.82:
        return S(A("INSERT"), gen_fn(rng, depth - 1))
    return rng.choice(_leaves(rng))


def test_metamorphic_sweep_lambda_equals_delta():
    rng = random.Random(20260702)
    for i in range(400):
        agree(gen_fn(rng, 3), gen_obj(rng, 3), i)


def test_theta_and_constraint_algebra_on_random_populations():
    rng = random.Random(42)
    col_eq = S(A("COMP"), A("eq"), S(A("CONS"), A(1), A(2)))          # rows with col1 = col2
    for i in range(40):
        pop = tuple(tuple(rng.choice("ab") for _ in range(3)) for _ in range(rng.randint(0, 6)))
        bin_pop = tuple(t[:2] for t in pop)
        for tag, f, x in [
            ("filter", T.Filter(col_eq), pop),
            ("project", T.Project([1, 3]), pop),
            ("tie", T.Tie, pop),
            ("uc", C.uniqueness([1]), pop),
            ("ring", C.ring_irreflexive((1, 2)), bin_pop),
            ("join", T.NatJoin(2), (bin_pop, bin_pop)),
            ("setminus", T.setminus, (pop, pop[: len(pop) // 2])),
        ]:
            agree(f, x, (tag, i))
