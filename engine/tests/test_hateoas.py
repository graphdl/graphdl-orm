"""Thm. hateoas: emit's representation carries links(e) = nav(e) ∪ transitions(status(e)) —
the related resources plus the actions available from the entity's current state."""
from pyarest import from_lam, to_lam
import pyarest.lam as L
import pyarest.prims  # noqa: F401
from pyarest import ast, system


def test_representation_carries_nav_and_transitions():
    # facts are ⟨entity, status⟩ (key at role 1, status at role 2); the SM relates statuses
    sm = to_lam((("pending", "approve", "active"), ("active", "close", "closed")))
    links = system.links_of(1, sm=sm, status_pos=2)
    D = L.SEQ(L.CONS(ast.cell("FILE", to_lam((("alice", "active"),))))(L.NIL))
    (o, _Dp) = from_lam(ast.run(to_lam(("alice", "pending")), D, links_obj=links))
    _p2, _v, lk = o
    # nav(e): the facts sharing alice's key
    assert ("alice", "pending") in lk and ("alice", "active") in lk
    # transitions(status(e)): from the head's status 'pending', only the pending transition
    assert ("pending", "approve", "active") in lk
    assert ("active", "close", "closed") not in lk


def test_links_default_to_navigation_without_a_machine():
    links = system.links_of(1)                               # no state machine → nav only
    D = L.SEQ(L.CONS(ast.cell("FILE", to_lam((("bob", "x"),))))(L.NIL))
    (o, _Dp) = from_lam(ast.run(to_lam(("bob", "y")), D, links_obj=links))
    _p2, _v, lk = o
    assert set(lk) == {("bob", "y"), ("bob", "x")}           # both bob facts, no transitions
