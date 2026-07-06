"""Phase 4 routing: the RMAP partition drives where a create lands (spec §4.4). An
absorbed fact type writes the entity's own cell, named noun:id as the reference engine
names it, updating its column of the 3NF row; holes are the default object '#'; a
conflicting functional write bottoms the row and the transition rule refuses the step
atomically. An own-table fact type keeps its per-fact-type cell. The absorbed fact
population is reassembled from the entity cells through the table's index."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


MODEL = """Order is an entity type.
Customer is an entity type.
Product is an entity type.
Date is a value type.
Order includes Product.
Each Order was placed on at most one Date.
Each Order is placed by exactly one Customer.
In each population of Order includes Product, each Order, Product combination occurs at most once.
"""


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return c[2]
    return None


def _routed(D, ft, fact):
    part = system.rmap_partition(D)
    return from_lam(system.create_routed(D, ft, to_lam(fact), part))


def test_routed_create_lands_in_the_entity_cell_with_holes():
    D, _ = forml.compile_model(MODEL)
    (o, Dp) = _routed(D, "Order_was_placed_on_Date", ("o1", "d1"))
    assert _cell(Dp, "Order:o1") == ("o1", "d1", "#")         # the row, customer still a hole
    assert ("o1",) in _cell(Dp, "Order")                      # the table index records the key
    (o2, Dp2) = _routed(to_lam(Dp), "Order_is_placed_by_Customer", ("o1", "c1"))
    assert _cell(Dp2, "Order:o1") == ("o1", "d1", "c1")       # the hole filled in place


def test_conflicting_functional_write_is_refused_atomically():
    D, _ = forml.compile_model(MODEL)
    (_o, Dp) = _routed(D, "Order_was_placed_on_Date", ("o1", "d1"))
    (o2, Dp2) = _routed(to_lam(Dp), "Order_was_placed_on_Date", ("o1", "d2"))
    assert o2 == "ERROR"                                      # the UC is structural now
    assert _cell(Dp2, "Order:o1") == ("o1", "d1", "#")        # and D is unchanged


def test_rewriting_the_same_value_is_idempotent():
    D, _ = forml.compile_model(MODEL)
    (_o, Dp) = _routed(D, "Order_was_placed_on_Date", ("o1", "d1"))
    (o2, Dp2) = _routed(to_lam(Dp), "Order_was_placed_on_Date", ("o1", "d1"))
    assert o2 != "ERROR" and _cell(Dp2, "Order:o1") == ("o1", "d1", "#")


def test_own_table_fact_types_route_unchanged():
    D, _ = forml.compile_model(MODEL)
    (_o, Dp) = _routed(D, "Order_includes_Product", ("o1", "p1"))
    assert ("o1", "p1") in _cell(Dp, "Order_includes_Product")


def test_ft_view_reassembles_the_population_from_entity_cells():
    D, _ = forml.compile_model(MODEL)
    part = system.rmap_partition(D)
    D = system.create_routed(D, "Order_was_placed_on_Date", to_lam(("o1", "d1")), part)
    D = apply(A(2), D)
    D = system.create_routed(D, "Order_is_placed_by_Customer", to_lam(("o2", "c2")), part)
    D = apply(A(2), D)
    assert system.ft_view(D, "Order_was_placed_on_Date", part) == {("o1", "d1")}
    assert system.ft_view(D, "Order_is_placed_by_Customer", part) == {("o2", "c2")}
