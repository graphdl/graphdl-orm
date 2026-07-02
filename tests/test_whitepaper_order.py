"""The whitepaper's §1 example, end-to-end (audit C1): the FORML 2 listing — entity types
with reference modes, fact types, the uniqueness reading, and the five state-machine
readings — compiles into M, and the machine RUNS: a POST creates an Order in 'In Cart';
the 'place' trigger fact advances it to 'Placed' in one AST step (Prop. onestep), after
which the representation offers 'ship' and no longer 'place'; 'ship' reaches 'Shipped'."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


MODEL = """Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Order is placed by Customer.
Each Order is placed by exactly one Customer.
Customer places Order.
Customer ships Order.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
Transition 'ship' is from Status 'Placed'.
Transition 'ship' is to Status 'Placed'.
Transition 'ship' is triggered by Fact Type 'Customer ships Order'.
"""
# (the whitepaper's ship goes to 'Shipped'; fixed below — kept here to mirror the listing shape)
MODEL = MODEL.replace("Transition 'ship' is to Status 'Placed'.", "Transition 'ship' is to Status 'Shipped'.")


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def _with_pop(D, name, pop):
    return apply(ast.Store(name), L.SEQ(L.CONS(to_lam(pop))(L.CONS(D)(L.NIL))))


def test_the_whitepaper_listing_parses_completely():
    _D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []                              # every reading is in the fragment


def test_sm_readings_populate_M():
    D, _ = forml.compile_model(MODEL)
    Dpy = from_lam(D)
    assert ("Order", "Order") in _cell(Dpy, "smDef")
    assert ("Order", "In Cart", "initial") in _cell(Dpy, "smStatus")
    assert ("place", "In Cart") in _cell(Dpy, "smFrom")
    assert ("place", "Placed") in _cell(Dpy, "smTo")
    assert ("place", "Customer_places_Order") in _cell(Dpy, "smTrigger")
    assert ("Order", "OrderId") in _cell(Dpy, "refMode")      # the (.OrderId) reference mode


def test_the_sm_triples_read_off_M():
    D, _ = forml.compile_model(MODEL)
    triples = set(system.sm_triples_named(D))
    assert ("place", "In Cart", "Customer_places_Order", "Placed") in triples
    assert ("ship", "Placed", "Customer_ships_Order", "Shipped") in triples


def test_place_then_ship_advances_the_machine_with_correct_links():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))    # POST /orders created o1

    triples = system.sm_triples(D)
    trans_of = system.transitions_of(to_lam(triples), 2)      # links: transitions from status(e)

    # links from 'In Cart': place offered, ship not (transitions_of reads the head fact of P)
    avail0 = from_lam(apply(trans_of, to_lam((("o1", "In Cart"),))))
    assert {t[1] for t in avail0} == {"Customer_places_Order"}

    # the 'place' trigger fact enters P — ONE AST step advances the machine (Prop. onestep);
    # the ORM layer attaches the machine from M, the caller names only the fact
    D = apply(A(2), system.create(D, "Customer_places_Order", to_lam(("c1", "o1"))))
    Dpy = from_lam(D)
    assert _cell(Dpy, "Order_status") == {("o1", "Placed")}   # advanced
    assert ("c1", "o1") in _cell(Dpy, "Customer_places_Order")

    # links from 'Placed': ship offered, place gone (Thm. hateoas: valid controls only)
    avail1 = from_lam(apply(trans_of, to_lam((("o1", "Placed"),))))
    assert {t[1] for t in avail1} == {"Customer_ships_Order"}

    # ship advances to Shipped
    D = apply(A(2), system.create(D, "Customer_ships_Order", to_lam(("c1", "o1"))))
    assert _cell(from_lam(D), "Order_status") == {("o1", "Shipped")}


def test_the_representation_itself_carries_the_changed_links():
    # §1: "following the place action advances the machine to Placed, after which THE
    # REPRESENTATION offers ship and no longer place" — o = ⟨P″, V, links(e)⟩ where the
    # links are computed from the entity's POST-step status, in the same reduction.
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    trans_of = system.transitions_of(to_lam(system.sm_triples(D)), 2)

    (o, Dp) = from_lam(ast.run(to_lam(("c1", "o1")), D, cell_name="Customer_places_Order",
                               machine=("Order_status", system.machine_step("Customer_places_Order"), 2),
                               links_obj=trans_of))
    _p2, _v, links = o
    assert {t[1] for t in links} == {"Customer_ships_Order"}  # ship offered, place gone

    # ship reaches Shipped — no outgoing transitions, so links(e) = φ: the paper's
    # logical deletion ("an entity that reaches a status with no outgoing transitions")
    D2 = _rebuild(Dp)
    (o2, _D3) = from_lam(ast.run(to_lam(("c1", "o1")), D2, cell_name="Customer_ships_Order",
                                 machine=("Order_status", system.machine_step("Customer_ships_Order"), 2),
                                 links_obj=trans_of))
    assert o2[2] == ()                                        # links(e) = φ — nothing left to do


def _rebuild(Dpy):
    """Scott D back from its from_lam projection (test convenience)."""
    return to_lam(Dpy)
