"""as_of moved into system.canon (Prop. onestep order_tau audit view; Halpin 13.6
bitemporal): the population as of transaction time tx is
  alpha(tl) . Filter(le(1, CONST(tx))) . FetchPop(log)
— keep log rows whose tx-column (role 1) is <= tx, then drop that column (tl),
leaving the fact. Gated ABSOLUTELY on a synthetic @tx log. The host supplies the
log cell name (ft+'@tx'; string concat is the seed boundary). Both hosts reduce the
same bytes (differential-covered primitives + ast:FetchPop)."""
import pyarest.prims  # noqa: F401
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

# @tx log rows are (tx, ...fact): a@1, b@2, c@3
D = to_lam((("CELL", "Fact@tx", ((1, "a"), (2, "b"), (3, "c"))),))


def as_of(log_cell, tx):
    expr = R(A("system:as_of"), to_lam((log_cell, tx)))
    return {tuple(r) for r in from_lam(R(expr, D))}


def test_as_of_2():
    # tx <= 2 keeps a,b; tl drops the tx column
    assert as_of("Fact@tx", 2) == {("a",), ("b",)}


def test_as_of_1():
    assert as_of("Fact@tx", 1) == {("a",)}


def test_as_of_3():
    assert as_of("Fact@tx", 3) == {("a",), ("b",), ("c",)}
