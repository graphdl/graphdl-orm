"""G3 seed — the paper's own example on the Python host (SPEC §13 G3).

AREST.tex §1's listing is normative: it must compile whole, its state
machine must assemble, and its transition triggers must resolve to the
DECLARED fact types across reading orientation ('Customer places Order'
names the fact type declared 'Order is placed by Customer' — one Verb,
two orientations). Same-bytes against the Rust host joins on Day 4.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

_APP = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                    "apps", "paper-order", "readings", "core.md")


def _compiled():
    from host_py import forml
    text = open(_APP, encoding="utf-8").read()
    return forml.compile_model(text)


def test_the_papers_listing_compiles_whole():
    D, rep = _compiled()
    assert rep.get("total") == 13
    assert not rep.get("unparsed"), rep.get("unparsed")
    assert not rep.get("prose"), rep.get("prose")


def test_triggers_resolve_across_reading_orientation():
    from host_py import system
    D, _ = _compiled()
    rows = set(system._pop_rows(D, "smTrigger"))
    assert rows == {("place", "Order_is_placed_by_Customer"),
                    ("ship", "Customer_ships_Order")}, rows


def test_the_machine_assembles_by_the_canonical_join():
    from host_py import system
    D, _ = _compiled()
    triples = set(system.sm_triples(D))
    assert triples == {("In Cart", "Order_is_placed_by_Customer", "Placed"),
                       ("Placed", "Customer_ships_Order", "Shipped")}, triples


def test_exactly_one_compiles_to_uc_and_mandatory():
    from host_py.lam import from_lam
    D, _ = _compiled()
    names = {c[1] for c in from_lam(D)
             if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"
             and isinstance(c[1], str)}
    assert "Order_is_placed_by_Customer_uc" in names
    assert "Order_is_placed_by_Customer_mand" in names
