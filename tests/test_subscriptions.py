"""Cor. stream, wired to the commit path: a subscription IS a ρ-application that has not
yet been evaluated against the current D — an ordinary named definition plus M facts
recording which cells it reads. step_and_wake is the commit path: one ORM-level create,
the semi-naive derivation of the affected fragment, then every subscription due on what
changed (transitively through the rule graph) evaluates against the new D. A
subscription on an untouched cell stays pending."""
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


MODEL = """Person is an entity type.
Person is a parent of Person.
Person1 is an ancestor of Person2 if Person1 is a parent of Person2.
Person1 is an ancestor of Person3 if Person1 is a parent of Person2 and Person2 is an ancestor of Person3.
"""

ANC = "Person_is_an_ancestor_of_Person"
PAR = "Person_is_a_parent_of_Person"


def test_subscriptions_wake_through_the_rule_graph():
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.DefineIn("anc_view", ast.FetchPop(ANC)), D)
    D = apply(ast.DefineIn("par_view", ast.FetchPop(PAR)), D)
    D = apply(ast.DefineIn("other_view", ast.FetchPop("Unrelated")), D)
    D = system.subscribe(D, "s_derived", [ANC], "anc_view")
    D = system.subscribe(D, "s_base", [PAR], "par_view")
    D = system.subscribe(D, "s_other", ["Unrelated"], "other_view")
    res, wakes = system.step_and_wake(D, PAR, to_lam(("a", "b")))
    assert set(wakes) == {"s_derived", "s_base"}              # due via the closure only
    assert ("a", "b") in set(wakes["s_derived"])              # sees the DERIVED facts
    assert ("a", "b") in set(wakes["s_base"])
    (_o, Dp) = from_lam(res)
    assert any(c[:2] == ("CELL", ANC) for c in Dp if isinstance(c, tuple) and len(c) == 3)


ORDER = """Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Customer places Order.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
"""


def test_a_refused_step_wakes_nothing():
    D, _ = forml.compile_model(ORDER)
    D = apply(ast.Store("Order_status"), S(to_lam((("o1", "In Cart"),)), D))
    D = apply(ast.DefineIn("v", ast.FetchPop("Customer_places_Order")), D)
    D = system.subscribe(D, "s", ["Customer_places_Order"], "v")
    # an atom fact bottoms the machine's dynamic role selection: §14.3.1 answers ERROR
    res, wakes = system.step_and_wake(D, "Customer_places_Order", to_lam("garbage"))
    assert from_lam(apply(A(1), res)) == "ERROR"
    assert wakes == {}                                        # ERROR commits nothing
