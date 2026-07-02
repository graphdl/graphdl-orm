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
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    (o, Dp) = from_lam(system.create(D, "Customer_places_Order", to_lam(("c1", "o1"))))
    assert ("c1", "o1") in _cell(Dp, "Customer_places_Order")
    assert _cell(Dp, "Order_status") == {("o1", "Placed")}    # no machine argument anywhere


def test_create_on_a_non_trigger_fact_type_has_no_machine_stage():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    (o, Dp) = from_lam(system.create(D, "Customer_greets_Customer", to_lam(("c1", "c2"))))
    assert ("c1", "c2") in _cell(Dp, "Customer_greets_Customer")
    assert _cell(Dp, "Order_status") == {("o1", "In Cart")}   # untouched


def test_mealy_emission_lands_in_the_representation():
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.DefineIn("place-receipt", S(A("CONST"), A("RECEIPT"))), D)
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    (o, _Dp) = from_lam(system.create(D, "Customer_places_Order", to_lam(("c1", "o1"))))
    assert o[-1] == (("o1", "RECEIPT"),)                      # ρ-applied on the fired step


def test_the_host_wired_path_is_gone():
    assert not hasattr(system, "sm_step")
    assert not hasattr(system, "machine_wiring")
