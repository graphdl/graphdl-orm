"""Phase 4 opening: RMAP read off M and driving D's layout (spec §4.4; whitepaper §Cells).
The partition comes from M's constraint facts through the one machine fold, absorption of
functional fact types into the key's 3NF row is a θ₁ join, and each entity gets its own
addressable cell, named as the reference TS engine names them (noun:id)."""
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


def _with_pop(D, name, pop):
    return apply(ast.Store(name), L.SEQ(L.CONS(to_lam(pop))(L.CONS(D)(L.NIL))))


def test_partition_is_read_off_M_not_prechewed():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    part = system.rmap_partition(D)
    assert part["Order_was_placed_on_Date"] == "Order"        # functional role: absorbed
    assert part["Order_is_placed_by_Customer"] == "Order"     # functional role: absorbed
    assert part["Order_includes_Product"] == "Order_includes_Product"   # spanning UC: own


def test_absorption_is_a_theta1_join_into_3NF_rows():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_was_placed_on_Date", (("o1", "d1"), ("o2", "d2")))
    D = _with_pop(D, "Order_is_placed_by_Customer", (("o1", "c1"), ("o2", "c2")))
    rows = system.absorb_rows(D, "Order", system.rmap_partition(D))
    assert set(rows) == {("o1", "d1", "c1"), ("o2", "d2", "c2")}


def test_each_entity_gets_its_own_addressable_cell():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_was_placed_on_Date", (("o1", "d1"),))
    D = _with_pop(D, "Order_is_placed_by_Customer", (("o1", "c1"),))
    rows = system.absorb_rows(D, "Order", system.rmap_partition(D))
    D2 = system.install_entity_cells(D, "Order", rows)
    assert from_lam(apply(ast.Fetch("Order:o1"), D2)) == ("o1", "d1", "c1")
    assert from_lam(apply(ast.Fetch("Order:o2"), D2)) == "#"  # absent entity unaddressable
