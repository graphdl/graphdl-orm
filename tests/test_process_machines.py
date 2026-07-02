"""A process IS a state machine definition in M (the whitepaper's §1 shape, completed):
statuses, initial, transitions with trigger fact types and GUARDS, and Mealy/Moore output
functions as named definitions resolved by ρ — all ORM metamodel facts, run by the one
fold. Guards are fact types (possibly DERIVED, so guard power = rule power) and therefore
positive: the groundedness condition on state transitions holds by construction. The
machine binds to a resource type or SUPERTYPE, governing subtype instances."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, defs, forml, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _with_pop(D, name, pop):
    return apply(ast.Store(name), S(to_lam(pop), D))


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


MODEL = """Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Product is an entity type.
Order includes Product.
Customer places Order.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
Transition 'place' is guarded by Fact Type 'Order includes Product'.
Transition 'place' emits 'place-receipt'.
Status 'Placed' emits 'awaiting-shipment'.
"""


def _step_place(D, fact):
    return from_lam(system.create(D, "Customer_places_Order", to_lam(fact)))


def test_guard_facts_parse_into_M():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    Dpy = from_lam(D)
    assert ("place", "Order_includes_Product") in _cell(Dpy, "smGuard")
    assert ("place", "place-receipt") in _cell(Dpy, "smEmit")
    assert ("Placed", "awaiting-shipment") in _cell(Dpy, "smMoore")


def test_unsatisfied_guard_blocks_the_transition_not_the_fact():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    (o, Dp) = _step_place(D, ("c1", "o1"))                    # o1 includes NO product
    assert ("c1", "o1") in _cell(Dp, "Customer_places_Order")  # the fact entered P
    assert _cell(Dp, "Order_status") == {("o1", "In Cart")}    # the machine did not fire


def test_satisfied_guard_fires_the_transition():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    D = _with_pop(D, "Order_includes_Product", (("o1", "p1"),))
    (_o, Dp) = _step_place(D, ("c1", "o1"))
    assert _cell(Dp, "Order_status") == {("o1", "Placed")}


DERIVED_GUARD = MODEL.replace(
    "Transition 'place' is guarded by Fact Type 'Order includes Product'.",
    "Order1 is ready if Order1 includes some Product1.\n"
    "Transition 'place' is guarded by Fact Type 'Order is ready'.")


def test_a_derived_fact_type_guards_with_rule_power():
    D, rep = forml.compile_model(DERIVED_GUARD)
    assert rep["unparsed"] == []
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    D = _with_pop(D, "Order_includes_Product", (("o1", "p1"),))
    D = system.run_rules(D)                                   # derive readiness first
    (_o, Dp) = _step_place(D, ("c1", "o1"))
    assert _cell(Dp, "Order_status") == {("o1", "Placed")}    # guarded by a DERIVED ft


SUB = """Party is an entity type.
Person is an entity type.
Person is a subtype of Party.
State Machine Definition 'Party' is for Noun 'Party'.
Status 'New' is initial in State Machine Definition 'Party'.
"""


def test_machine_binds_through_the_supertype_chain():
    D, _ = forml.compile_model(SUB)
    assert system.machine_for(D, "Party") == "Party"
    assert system.machine_for(D, "Person") == "Party"         # subtype governed by the
    assert system.machine_for(D, "Unbound") is None           # supertype's machine


def test_moore_emission_is_a_rho_application_over_the_status():
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.DefineIn("awaiting-shipment", S(A("CONST"), A("SHIP-SOON"))), D)
    D = _with_pop(D, "Order_status", (("o1", "Placed"),))
    view = system.moore_view(D, "Order")
    assert view[("o1", "Placed")] == "SHIP-SOON"
