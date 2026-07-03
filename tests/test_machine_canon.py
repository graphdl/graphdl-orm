"""Wave S3: the M-driven machine from the shared source. machine_step and mealy_step
are two projections of one in-step expression over ⟨statusPop, P″, D⟩ that joins the
trigger's transitions, guards, and emissions from M INSIDE the reduction and reads
the governed player's role position the same way, so editing M redirects the step
with no rewiring — that property must survive the migration byte for byte.
transitions_of is the HATEOAS leg: the transitions available from the head fact's
status, over the M-read triples as data. The gates run the canonical names against
the host builders on the whitepaper ORDER machine, with the absolute expectations
the machine suites already pin."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import ast, defs, forml, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


ORDER = """Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Customer places Order.
Customer ships Order.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
Transition 'ship' is from Status 'Placed'.
Transition 'ship' is to Status 'Shipped'.
Transition 'ship' is triggered by Fact Type 'Customer ships Order'.
"""


def _fixture():
    D, _ = forml.compile_model(ORDER)
    D = apply(ast.Store("Order_status"), S(to_lam((("o1", "In Cart"),)), D))
    spop = to_lam((("o1", "In Cart"),))
    P2 = to_lam((("c1", "o1"),))                              # the trigger fact entered P''
    return D, S(spop, P2, D)


def test_machine_step_advances_from_the_canon():
    D, x = _fixture()
    host = system.machine_step("Customer_places_Order")
    with defs.step(D):
        built = apply(A("system:machine_step"),
                      S(A("Customer_places_Order"), to_lam(())))
        got = from_lam(apply(built, x))
        want = from_lam(apply(host, x))
    assert got == want
    assert got == (("o1", "Placed"),)                         # the transition fired


def test_machine_step_holds_without_a_trigger():
    D, _ = _fixture()
    x = S(to_lam((("o1", "In Cart"),)), to_lam(()), D)        # nothing entered P''
    host = system.machine_step("Customer_places_Order")
    with defs.step(D):
        built = apply(A("system:machine_step"),
                      S(A("Customer_places_Order"), to_lam(())))
        got = from_lam(apply(built, x))
        want = from_lam(apply(host, x))
    assert got == want == (("o1", "In Cart"),)                # unchanged status


def test_mealy_step_emits_nothing_on_silent_transitions():
    D, x = _fixture()
    host = system.mealy_step("Customer_places_Order")
    with defs.step(D):
        built = apply(A("system:mealy_step"),
                      S(A("Customer_places_Order"), to_lam(())))
        got = from_lam(apply(built, x))
        want = from_lam(apply(host, x))
    assert got == want == ()                                  # no smEmit declared: silence


def test_transitions_of_offers_exactly_the_next_actions():
    D, _ = _fixture()
    sm = to_lam(system.sm_triples(D))
    host = system.transitions_of(sm, 2)
    with defs.step(D):
        built = apply(A("system:transitions_of"), S(sm, A(2)))
        placed = from_lam(apply(built, to_lam((("o1", "Placed"),))))
        want = from_lam(apply(host, to_lam((("o1", "Placed"),))))
    assert placed == want
    # the offered trigger is the SHIPS fact type, and no longer places (§1)
    assert [t[1] for t in placed] == ["Customer_ships_Order"]
