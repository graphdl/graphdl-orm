"""Fifth triage batch: evaluate.rs's initial-status scenarios. The paper's machine is
machine(s_0, E) = foldl transition s_0 (order_tau E) — s_0 is a DECLARED parameter
(the corpus surface `Status ... is initial in State Machine Definition ...` appears in
the paper's own Definition example). The old engine additionally INFERRED s_0 from
graph topology (the unique source-never-target status) when undeclared, refusing only
the cyclic case ("no insertion-order fallback"). Verdict: the inference is
engine-invented; the paper takes s_0 as data, so the canonical behavior for an
undeclared initial is HONEST ABSENCE, uniformly — the triggering fact commits (it is
a legal fact regardless), no status row lands, and nothing guesses from topology or
insertion order. The explicit-declaration scenario is the canonical one and is
already covered by the machine suites."""
import pyarest.prims  # noqa: F401
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import forml, system
from pyarest.reduce import apply


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_machine_without_declared_initial_lands_no_status_row():
    MODEL = """Order(.id) is an entity type.
Customer(.Name) is an entity type.
Customer ships Order.
State Machine Definition 'Order' is for Noun 'Order'.
Transition 'ship' is from Status 'Placed'.
Transition 'ship' is to Status 'Shipped'.
Transition 'ship' is triggered by Fact Type 'Customer ships Order'.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    D = system.layout_cells(system.status_facts(D))          # status(e): RMAP column
    res = system.create(D, "Customer_ships_Order", to_lam(("c1", "o1")))
    o, D2 = from_lam(apply(A(1), res)), apply(A(2), res)
    assert o != "ERROR"                                       # the fact is legal data
    Dpy = from_lam(D2)
    assert _cell(Dpy, "Customer_ships_Order") == {("c1", "o1")}
    # no s_0 declared: the fold has no seed, so NOTHING advances — no topology
    # inference, no insertion-order guess, no status row in the column
    assert system.ft_view(D2, "Order_is_currently_in_Status",
                          system.rmap_partition(D2)) == set()


def test_noun_without_a_machine_takes_plain_facts():
    MODEL = """Customer(.Name) is an entity type.
Order(.id) is an entity type.
Customer places Order.
"""
    D, _ = forml.compile_model(MODEL)
    res = system.create(D, "Customer_places_Order", to_lam(("c1", "o1")))
    o, D2 = from_lam(apply(A(1), res)), apply(A(2), res)
    assert o != "ERROR"
    Dpy = from_lam(D2)
    assert _cell(Dpy, "Customer_places_Order") == {("c1", "o1")}
    assert not any(isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"
                   and str(c[1]).endswith("_status") for c in Dpy)
