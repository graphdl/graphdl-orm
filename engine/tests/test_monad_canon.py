"""The polyglot monad, gated strictly by the three monad laws applied to the
canonical NAMES from shared/system.canon. M a = <a, log> is the Writer over the
free monoid (log = sequence, mempty = phi, mappend = cat); unit = CONS[id,
CONST(phi)] and bind threads the pair through apply + cat. The laws are proven
here in the Python reducer; because unit/bind compose only differential-covered
primitives (COMP, CONS, CONST, apply, cat, id, selectors), the Rust host reduces
the identical bytes to the identical normal forms (system.canon is include!d).
The joke and the discipline are the same fact: what language is it? Yes."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import from_lam, atom as A
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


UNIT, BIND = A("monad:unit"), A("monad:bind")


def unit(v):
    return apply(UNIT, v)


def bind(mv, k):
    return apply(BIND, S(mv, k))


# Kleisli arrows a -> M b, as closed FFP objects:
F = S(A("CONS"), A("id"), S(A("CONS"), A("id")))                # f a = <a, <a>>
G = S(A("CONS"), A("id"), S(A("CONS"), A("id"), A("id")))       # g b = <b, <b, b>>


def test_left_identity():
    # bind(unit a, f) == f a
    a = A("x")
    assert from_lam(bind(unit(a), F)) == from_lam(apply(F, a))


def test_right_identity():
    # bind(m, unit) == m
    m = S(A("x"), S(A("w1"), A("w2")))
    assert from_lam(bind(m, UNIT)) == from_lam(m)


def test_associativity():
    # bind(bind(m, f), g) == bind(m, \x. bind(f x, g))
    m = S(A("x"), S(A("w1"), A("w2")))
    h = S(A("COMP"), BIND, S(A("CONS"), F, S(A("CONST"), G)))   # h x = bind(f x, g)
    assert from_lam(bind(bind(m, F), G)) == from_lam(bind(m, h))
