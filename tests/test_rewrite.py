"""The Backus-level rewriter, v1: the unconditionally ⊥-safe laws (composition
associativity, III.2 identity elimination, II.3.1 redundant-test elimination) over
object trees, and the twin oracle holding a rewritten object to observational
equality on given operands — a diverging twin raises, because a twin that diverges
is a bug, not a fallback. The qualified laws (I.5 and friends) wait for the operand
oracle per the catalog."""
import pytest
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import rewrite
from pyarest import canon as theta
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def test_composition_flattens_and_id_drops():
    t = ("COMP", ("COMP", "eq", ("COMP", "not", "not")), "id", ("COMP", "tl"))
    assert rewrite.rewrite(t) == ("COMP", "eq", "not", "not", "tl")
    assert rewrite.rewrite(("COMP", "id", "id")) == "id"
    assert rewrite.rewrite(("COMP", "length")) == "length"


def test_redundant_cond_test_collapses():
    t = ("COND", "null", ("COND", "null", "length", "tl"), "reverse")
    assert rewrite.rewrite(t) == ("COND", "null", "length", "reverse")


def test_twin_holds_the_oracle_and_answers_the_rewrite():
    obj = S(A("COMP"), S(A("COMP"), A("not"), A("null")), A("id"))
    ops = [to_lam(()), to_lam((1, 2))]
    tw = rewrite.twin(obj, ops)
    assert from_lam(tw) == ("COMP", "not", "null")
    for x in ops:
        assert from_lam(apply(tw, x)) == from_lam(apply(obj, x))


def test_twin_raises_on_divergence():
    class Evil:
        pass
    # force divergence by rewriting a tree whose flattening is NOT equal in this
    # kernel: none exists among the safe laws, so simulate via a monkeypatched law
    real = rewrite.rewrite
    rewrite.rewrite = lambda t: "length" if t == ("COMP", "not", "null") else real(t)
    try:
        with pytest.raises(AssertionError):
            rewrite.twin(S(A("COMP"), A("not"), A("null")), [to_lam((1,))])
    finally:
        rewrite.rewrite = real


def test_a_real_built_object_shrinks_and_agrees():
    j = theta.NatJoin(2)                                      # canon-built, ref-bearing
    ops = [S(to_lam((("a", 1),)), to_lam(((1, "z"),))),
           S(to_lam(()), to_lam(((1, "z"),)))]
    tw = rewrite.twin(j, ops)
    before = repr(from_lam(j))
    after = repr(from_lam(tw))
    assert len(after) <= len(before)                          # never grows
