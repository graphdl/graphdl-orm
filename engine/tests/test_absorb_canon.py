"""absorb (RMAP 3NF, spec Sec 4.4) moved into system.canon: the 3NF row population
folds theta:NatJoin over a VARIABLE number of absorbed fact-type populations, joining
on the shared key (role 1, the id):
  system:absorb_core : <key, <p1..pn>> = INSERT[theta:NatJoin:key] : <p1..pn>
The variadic-ness IS the INSERT fold; the single-population case is INSERT's identity.
Gated against the expected join AND against the host absorb_rows fold. Both hosts
reduce the same bytes (theta:NatJoin + INSERT + apply, all differential-covered)."""
import pyarest.prims  # noqa: F401
from pyarest import canon as T
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R
import pyarest.lam as L

# three 3NF fact-type populations keyed on role 1 (the id); 'c' has no p1/p2 row
P0 = (("a", 1), ("b", 2), ("c", 9))
P1 = (("a", "x"), ("b", "y"))
P2 = (("a", "P"), ("b", "Q"))


def _seq(rows):
    return to_lam(tuple(tuple(r) for r in rows))


def _popseq(pops):
    s = L.NIL
    for r in reversed(pops):
        s = L.CONS(_seq(r))(s)
    return L.SEQ(s)


def _canon(pops, key=1):
    inp = L.SEQ(L.CONS(A(key))(L.CONS(_popseq(pops))(L.NIL)))
    return [tuple(r) for r in from_lam(R(A("system:absorb_core"), inp))]


def _host(pops):
    acc = _seq(pops[0])
    for nxt in pops[1:]:
        acc = R(T.NatJoin(1), L.SEQ(L.CONS(acc)(L.CONS(_seq(nxt))(L.NIL))))
    return [tuple(r) for r in from_lam(acc)]


def test_absorb_three():
    # inner join on the key: 'c' drops (no p1/p2), rows widen id+val1+val2+val3
    assert _canon((P0, P1, P2)) == [("a", 1, "x", "P"), ("b", 2, "y", "Q")]


def test_absorb_two():
    assert _canon((P0, P1)) == [("a", 1, "x"), ("b", 2, "y")]


def test_absorb_one_is_identity():
    # INSERT base case: a single population folds to itself
    assert _canon((P0,)) == [("a", 1), ("b", 2), ("c", 9)]


def test_absorb_canon_equals_host():
    for pops in [(P0, P1, P2), (P0, P1), (P0,), (P1, P2)]:
        assert _canon(pops) == _host(pops)
