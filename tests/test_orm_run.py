"""The ORM layer runs the machines; the AST layer is how they run (Prop. onestep). The
caller names only the fact: system.create reads off M whether the fact type triggers a
machine and which noun it governs, attaches the M-driven step automatically, and the
transition fires because the trigger fact enters P — no machine argument anywhere.
Mealy emissions ride the same step: the transition's named definition, resolved by ρ,
applied to ⟨entity, from, to⟩, lands in the representation. The host-wired path
(sm_step, machine_wiring) is deleted."""
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


def _setup(model):
    D, _ = forml.compile_model(model)
    return system.layout_cells(system.status_facts(D))       # status(e): RMAP column


def _create(D, ft, *rows):
    for row in rows:
        D = apply(A(2), system.create(D, ft, to_lam(row)))
    return D


def _status(D, ft="Order_is_currently_in_Status"):
    return system.ft_view(D, ft, system.rmap_partition(D))


MODEL = """Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Customer places Order.
Customer greets Customer.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
Transition 'place' emits 'place-receipt'.
"""


def test_create_runs_the_machine_from_M_alone():
    D = _setup(MODEL)
    D = _create(D, "Order_is_currently_in_Status", ("o1", "In Cart"))
    (o, Dp) = from_lam(system.create(D, "Customer_places_Order", to_lam(("c1", "o1"))))
    assert ("c1", "o1") in _cell(Dp, "Customer_places_Order")
    assert ("o1", "Placed") in _status(to_lam(Dp))            # no machine argument anywhere


def test_create_on_a_non_trigger_fact_type_has_no_machine_stage():
    D = _setup(MODEL)
    D = _create(D, "Order_is_currently_in_Status", ("o1", "In Cart"))
    (o, Dp) = from_lam(system.create(D, "Customer_greets_Customer", to_lam(("c1", "c2"))))
    assert ("c1", "c2") in _cell(Dp, "Customer_greets_Customer")
    assert ("o1", "In Cart") in _status(to_lam(Dp))           # untouched


def test_mealy_emission_lands_in_the_representation():
    D = _setup(MODEL)
    D = apply(ast.DefineIn("place-receipt", S(A("CONST"), A("RECEIPT"))), D)
    D = _create(D, "Order_is_currently_in_Status", ("o1", "In Cart"))
    (o, _Dp) = from_lam(system.create(D, "Customer_places_Order", to_lam(("c1", "o1"))))
    assert o[-1] == (("o1", "RECEIPT"),)                      # ρ-applied on the fired step


def test_the_host_wired_path_is_gone():
    assert not hasattr(system, "sm_step")
    assert not hasattr(system, "machine_wiring")


def _rows(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return c[2]                                       # raw rows, duplicates visible
    return ()


FLOW = MODEL + """Customer ships Order.
Transition 'ship' is from Status 'Placed'.
Transition 'ship' is to Status 'Shipped'.
Transition 'ship' is triggered by Fact Type 'Customer ships Order'.
"""


def test_create_carries_the_links_with_no_caller_wiring():
    # Thm. hateoas at the ORM level: the representation offers exactly the next
    # transitions from the entity's POST-step status — no links argument anywhere
    D = _setup(FLOW)
    D = _create(D, "Order_is_currently_in_Status", ("o1", "In Cart"))
    (o, _Dp) = from_lam(system.create(D, "Customer_places_Order", to_lam(("c1", "o1"))))
    links = o[2]
    assert {t[1] for t in links} == {"Customer_ships_Order"}  # ship offered, place gone


def test_reasserting_a_fact_is_the_identity_on_D():
    # fact-as-function: a population is a set, so re-assertion is the identity. This is
    # the ground case of the schema-derived failure model: at-least-once delivery is
    # free for asserts, and machine double-fire is structurally safe because firing
    # consumes the FROM status (a re-delivered trigger finds no transition to take).
    D = _setup(MODEL)
    D = _create(D, "Order_is_currently_in_Status", ("o1", "In Cart"))
    D1 = apply(A(2), system.create(D, "Customer_places_Order", to_lam(("c1", "o1"))))
    D2 = apply(A(2), system.create(D1, "Customer_places_Order", to_lam(("c1", "o1"))))
    D2py = from_lam(D2)
    assert _rows(D2py, "Customer_places_Order") == (("c1", "o1"),)   # once, not twice
    assert ("o1", "Placed") in _status(D2)                           # no double-fire
