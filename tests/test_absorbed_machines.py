"""Machines on ABSORBED trigger fact types: a functional trigger (single-role UC) routes
to its RMAP table, writing the entity's own 3NF row, and the machine advances in the
SAME step. The fired-check is the row form: the trigger's column went non-hole and the
addressed entity is the row's key (or the column value when the governed noun plays role
2; the position is still read from M in-step). The caller still names only the fact."""
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
Order is paid by Customer.
Each Order is paid by exactly one Customer.
Order is noted by Customer.
Each Order is noted by exactly one Customer.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'pay' is from Status 'In Cart'.
Transition 'pay' is to Status 'Paid'.
Transition 'pay' is triggered by Fact Type 'Order is paid by Customer'.
"""


def test_an_absorbed_trigger_advances_the_machine_in_the_routed_step():
    D, _ = forml.compile_model(MODEL)
    part = system.rmap_partition(D)
    assert part["Order_is_paid_by_Customer"] == "Order"       # functional: absorbed
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    res = system.create(D, "Order_is_paid_by_Customer", to_lam(("o1", "c1")))
    Dp = apply(A(2), res)
    Dpy = from_lam(Dp)
    assert system.ft_view(Dp, "Order_is_paid_by_Customer", part) == {("o1", "c1")}
    assert _cell(Dpy, "Order_status") == {("o1", "Paid")}     # advanced in the same step


def test_an_absorbed_non_trigger_leaves_the_machine_alone():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Order_status", (("o1", "In Cart"),))
    Dp = apply(A(2), system.create(D, "Order_is_noted_by_Customer", to_lam(("o1", "c1"))))
    assert _cell(from_lam(Dp), "Order_status") == {("o1", "In Cart")}
