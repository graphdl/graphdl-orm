"""The delta fast-path is observationally equal to the lambda kernel (spec §4.2 / D5): for every
FFP object, reduce.apply (delta) agrees with reduce.apply_lambda (mu = Y(tau), the ground truth).
The whole suite already runs through delta; this asserts the equivalence deliberately."""
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import reduce as R
import pyarest.prims  # noqa: F401
from pyarest import theta as T, constraints as C, system


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _agree(f, x):
    xl = to_lam(x)
    return from_lam(R.apply_lambda(f, xl)) == from_lam(R.apply(f, xl))


def test_delta_equals_lambda_across_the_algebra():
    cases = [
        (A(1), ("a", "b", "c")),                                     # a selector
        (T.Filter(A("eq")), (("a", "a"), ("a", "b"), ("c", "c"))),   # Codd selection
        (T.Project([1]), (("a", "x"), ("b", "y"), ("a", "z"))),      # projection (dedup)
        (T.NatJoin(2), ((("a", "1"), ("b", "2")), (("1", "x"), ("2", "y")))),  # natural join
        (T.Tie, (("a", "b", "a"), ("c", "d", "e"))),                 # tie
        (C.uniqueness([1]), (("a", "x"), ("b", "y"), ("a", "z"))),   # a constraint (rho c):P
        (system.join_rule(2, [1, 3]), (("a", "b"), ("b", "c"))),     # a derivation rule
        (system.derive_of([system.join_rule(2, [1, 3])]),
         (("a", "b"), ("b", "c"), ("c", "d"))),                      # derive = lfp F_S (transitive closure)
    ]
    for f, x in cases:
        assert _agree(f, x), (from_lam(f) if False else x)


def test_delta_bottom_and_metacomposition():
    assert _agree(A(4), ("a", "b"))                                  # out-of-range selector -> bottom
    comp = _S(A("COMP"), A(1), A("tl"))                            # metacomposition ⟨COMP,1,tl⟩
    assert _agree(comp, ("a", "b", "c"))
