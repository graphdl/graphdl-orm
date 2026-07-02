"""Testable consequences of the writer model (docs/2026-07-02-writer-model.md).

1. Steps on disjoint cells commute (level 0/1 serializability: commuting steps interleave
   freely, so any per-stream-consistent linearization is a serial witness).
2. Order decides survival exactly at an alethic scope (the double-spend vignette: "the
   earliest transaction is the one that counts", Nakamoto §2), and the loser is refused
   atomically in either order.
3. Derivation commutes with merge (CALM/CRDT confluence: lfp is a closure operator, so
   lfp(F, A ∪ B) = lfp(F, lfp(F,A) ∪ lfp(F,B))) — the monotone fragment needs no writer.
"""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


def _D(*cells):
    l = L.NIL
    for c in reversed(cells):
        l = L.CONS(c)(l)
    return L.SEQ(l)


def _cellset(Dpy):
    return {(c[1], tuple(c[2]) if isinstance(c[2], tuple) else c[2])
            for c in Dpy if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"}


def _step(D, cell, fact):
    return apply(A(2), ast.run(to_lam(fact), D, cell_name=cell))


def test_disjoint_cell_steps_commute():
    D0 = _D(ast.cell("A", to_lam(())), ast.cell("B", to_lam(())))
    D_ab = _step(_step(D0, "A", ("a1", "x")), "B", ("b1", "y"))
    D_ba = _step(_step(D0, "B", ("b1", "y")), "A", ("a1", "x"))
    assert _cellset(from_lam(D_ab)) == _cellset(from_lam(D_ba))


MODEL = """Order is an entity type.
Customer is an entity type.
Date is a value type.
Each Order was placed on at most one Date.
Each Order is placed by exactly one Customer.
"""


def test_order_decides_survival_at_the_alethic_scope():
    # two conflicting functional writes: in EITHER order, the first commits and the
    # second is refused atomically — order matters exactly here, so this scope (and only
    # this scope) needs one stream
    D, _ = forml.compile_model(MODEL)
    part = system.rmap_partition(D)
    for first, second in ((("o1", "d1"), ("o1", "d2")), (("o1", "d2"), ("o1", "d1"))):
        (o1, D1) = from_lam(system.create_routed(D, "Order_was_placed_on_Date", to_lam(first), part))
        assert o1 != "ERROR"
        (o2, D2) = from_lam(system.create_routed(to_lam(D1), "Order_was_placed_on_Date",
                                                 to_lam(second), part))
        assert o2 == "ERROR"                                  # the earliest one counts
        row = [c[2] for c in D2 if isinstance(c, tuple) and c[:2] == ("CELL", "Order:o1")][0]
        assert row == ("o1", first[1], "#")                   # and the state is unchanged


def test_derivation_commutes_with_merge():
    # the monotone fragment is coordination-free: closure of a union equals the closure
    # of the union of closures (transitive-closure rule as F_S)
    derive = system.derive_of([system.join_rule(2, [1, 3])])
    A_pop = (("a", "b"), ("b", "c"))
    B_pop = (("c", "d"),)
    together = set(from_lam(apply(derive, to_lam(A_pop + B_pop))))
    da = from_lam(apply(derive, to_lam(A_pop)))
    db = from_lam(apply(derive, to_lam(B_pop)))
    merged = set(from_lam(apply(derive, to_lam(tuple(da) + tuple(db)))))
    assert together == merged
    assert ("a", "d") in together                             # the closure genuinely crossed
