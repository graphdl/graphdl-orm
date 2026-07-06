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


def _setup(model):
    D, _ = forml.compile_model(model)
    return system.layout_cells(system.status_facts(D))       # status(e) as its RMAP column


def _create(D, ft, *rows):
    for row in rows:
        D = apply(A(2), system.create(D, ft, to_lam(row)))
    return D


def _status(D, ft):
    return system.ft_view(D, ft, system.rmap_partition(D))


def test_the_machine_runs_from_M_with_no_host_wiring():
    D = _setup(MODEL)
    D = _create(D, "Order_is_currently_in_Status", ("o1", "In Cart"))
    D = _create(D, "Customer_places_Order", ("c1", "o1"))
    assert ("o1", "Placed") in _status(D, "Order_is_currently_in_Status")


def test_editing_M_redirects_the_running_machine():
    D = _setup(MODEL)
    D = _create(D, "Order_is_currently_in_Status", ("o1", "In Cart"))
    D = _with_pop(D, "smTo", (("place", "Held"),))            # rewrite the machine IN M
    D = _create(D, "Customer_places_Order", ("c1", "o1"))
    assert ("o1", "Held") in _status(D, "Order_is_currently_in_Status")   # same step obeys


def test_a_guard_added_in_M_applies_with_no_rewiring():
    D = _setup(MODEL)
    D = _create(D, "Order_is_currently_in_Status", ("o1", "In Cart"))
    D = _with_pop(D, "smGuard", (("place", "Order_is_paid"),))
    D1 = _create(D, "Customer_places_Order", ("c1", "o1"))
    assert ("o1", "In Cart") in _status(D1, "Order_is_currently_in_Status")   # unpaid: blocks
    D2 = _with_pop(D, "Order_is_paid", (("o1",),))
    D2 = _create(D2, "Customer_places_Order", ("c1", "o1"))
    assert ("o1", "Placed") in _status(D2, "Order_is_currently_in_Status")    # paid: fires


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
    D = system.layout_cells(system.status_facts(D))          # Party's status column
    assert ("Person", "Party") in _cell(from_lam(D), "governedBy")   # governed via chain
    D = _create(D, "Party_is_currently_in_Status", ("p1", "New"))
    D = _create(D, "Person_signs_Agreement", ("p1", "a1"))
    assert ("p1", "Engaged") in _status(D, "Party_is_currently_in_Status")  # role via closure
