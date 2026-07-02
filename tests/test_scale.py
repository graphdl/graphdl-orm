"""ARC-scale engineering (strata map [~~]): the runtime path's folds must not consume one
host stack frame per element. INSERT is a right fold and WHILE is tail recursion, so both
have iterative forms with identical semantics (the oracle guards equality); under a
lowered recursion limit the recursive forms die and the iterative forms do not. The λ
kernel stays recursive; it is the ground truth, not the runtime."""
import sys
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import reduce as R


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def test_runtime_folds_do_not_recurse_per_element():
    plus_fold = S(A("INSERT"), A("+"))
    counter = S(A("WHILE"),
                S(A("COMP"), A("lt"), S(A("CONS"), A("id"), S(A("CONST"), A(20000)))),
                S(A("COMP"), A("+"), S(A("CONS"), A("id"), S(A("CONST"), A(1)))))
    old = sys.getrecursionlimit()
    sys.setrecursionlimit(3000)
    try:
        assert from_lam(R.apply(plus_fold, to_lam(tuple(range(5000))))) == sum(range(5000))
        assert from_lam(R.apply(counter, to_lam(0))) == 20000
    finally:
        sys.setrecursionlimit(old)
