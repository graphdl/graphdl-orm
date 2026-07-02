"""The machine that runs IS the M-facts (no Python-assembled copy): machine_step reads
transitions, guards, and the entity's role position from D inside the reduction — the
θ₁ join over smFrom/smTrigger/smTo/smGuard computed in-step, the role position looked up
from M's role facts in-step, and the position applied as a DYNAMIC selector (numbers are
selectors, so apply:⟨pos, fact⟩ selects at a runtime-computed role). The proof that M is
live: editing an smTo fact redirects the running machine; adding a guard fact applies to
the next step with no rewiring; and supertype governance flows through the derived
governedBy closure produced by the engine's own rule runner."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, system
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
Customer places Order.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
"""


def _step(D, fact):
    return from_lam(ast.run(to_lam(fact), D, cell_name="Customer_places_Order",
                            machine=("Order_status", system.machine_step("Customer_places_Order"))))


def test_the_machine_runs_from_M_with_no_host_wiring():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    (_o, Dp) = _step(D, ("c1", "o1"))
    assert _cell(Dp, "Order_status") == {("o1", "Placed")}


def test_editing_M_redirects_the_running_machine():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    D = _with_pop(D, "smTo", (("place", "Held"),))            # rewrite the machine IN M
    (_o, Dp) = _step(D, ("c1", "o1"))
    assert _cell(Dp, "Order_status") == {("o1", "Held")}      # the same step object obeys


def test_a_guard_added_in_M_applies_with_no_rewiring():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    D = _with_pop(D, "smGuard", (("place", "Order_is_paid"),))
    (_o, Dp) = _step(D, ("c1", "o1"))
    assert _cell(Dp, "Order_status") == {("o1", "In Cart")}   # unpaid: guard blocks
    D2 = _with_pop(D, "Order_is_paid", (("o1",),))
    (_o2, Dp2) = _step(D2, ("c1", "o1"))
    assert _cell(Dp2, "Order_status") == {("o1", "Placed")}   # paid: fires


SUPER = """Party is an entity type.
Person is an entity type.
Person is a subtype of Party.
Agreement(.Nr) is an entity type.
Person signs Agreement.
State Machine Definition 'Party' is for Noun 'Party'.
Status 'New' is initial in State Machine Definition 'Party'.
Transition 'engage' is from Status 'New'.
Transition 'engage' is to Status 'Engaged'.
Transition 'engage' is triggered by Fact Type 'Person signs Agreement'.
"""


def test_supertype_governance_via_the_derived_closure():
    D, _ = forml.compile_model(SUPER)
    D = system.governance_rules(D)                            # governedBy, by the engine's
    D = system.run_rules(D)                                   # own rule runner
    Dpy = from_lam(D)
    assert ("Person", "Party") in _cell(Dpy, "governedBy")    # subtype governed via chain
    D = _with_pop(D, "Person_status", (("p1", "New"),))
    (_o, Dp) = from_lam(ast.run(to_lam(("p1", "a1")), D, cell_name="Person_signs_Agreement",
                                machine=("Person_status", system.machine_step("Person_signs_Agreement"))))
    assert _cell(Dp, "Person_status") == {("p1", "Engaged")}  # Person role found via closure
