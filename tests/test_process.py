"""Process management in the AST context, from the sources:

§14.3.2 verbatim: the system accepts ⟨RESET, x⟩ at any time; if SYSTEM is defined it
"aborts its current computation without altering D" and treats x as new input; if SYSTEM
is undefined, x is appended to D (the bootstrap). §14.4.3's installation sequence is the
test. Cor. boundary implies the supervisor: compiled steps terminate (Lemma finiteness),
so supervision is needed exactly at the undecidable surface — fuel-bounded reduction
turns a runaway step into §14.3.1's ⟨ERROR, unchanged D⟩. And the run queue is a VIEW:
an entity whose status has outgoing transitions is a waiting process (its awaited events
are its triggers); one with none has terminated (links = φ, the paper's deletion)."""
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


def _D(*cells):
    l = L.NIL
    for c in reversed(cells):
        l = L.CONS(c)(l)
    return L.SEQ(l)


def test_reset_bootstrap_replays_backus_14_4_3():
    # loader = [DONE̅, id]; RESET installs it as SYSTEM in the empty store; the next input
    # D0 becomes the state and DONE is the output — Backus's installation, verbatim
    loader = S(A("CONS"), S(A("CONST"), A("DONE")), A("id"))
    D = ast.reset(ast.cell("SYSTEM", loader), _D())
    D0 = to_lam((("defs", "would", "go", "here"),))
    (o, d) = from_lam(ast.step_input(D0, D))
    assert o == "DONE" and d == (("defs", "would", "go", "here"),)


def test_reset_with_system_defined_treats_x_as_normal_input():
    loader = S(A("CONS"), S(A("CONST"), A("DONE")), A("id"))
    D = ast.reset(ast.cell("SYSTEM", loader), _D())
    (o, _d) = from_lam(ast.reset(to_lam(("x",)), D))          # SYSTEM defined: normal input
    assert o == "DONE"                                        # and D was not altered first


def test_fuel_bounds_a_runaway_step():
    # a compiled infinite loop: (while T̄ id) — the finiteness hypothesis violated on
    # purpose; under fuel the step answers ⟨ERROR, unchanged D⟩ instead of hanging
    spin = S(A("WHILE"), S(A("CONST"), A("T")), A("id"))
    D = _D(ast.cell("FILE", to_lam(())))
    (o, d) = from_lam(ast.run(to_lam(("a",)), D, derive_obj=spin, fuel=10000))
    assert o == "ERROR" and d == from_lam(D)


def test_fuel_does_not_disturb_a_terminating_step():
    D = _D(ast.cell("FILE", to_lam(())))
    (o, Dp) = from_lam(ast.run(to_lam(("a", "x")), D, fuel=200000))
    assert ("a", "x") in o[0]


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


def test_the_run_queue_is_a_view():
    D, _ = forml.compile_model(ORDER)
    D = system.layout_cells(system.status_facts(D))          # status(e): RMAP column
    for row in (("o1", "In Cart"), ("o2", "Placed"), ("o3", "Shipped")):
        D = apply(A(2), system.create(D, "Order_is_currently_in_Status", to_lam(row)))
    table = system.process_table(D, "Order")
    assert table[("o1", "In Cart")] == ("Customer_places_Order",)   # awaiting place
    assert table[("o2", "Placed")] == ("Customer_ships_Order",)     # awaiting ship
    assert ("o3", "Shipped") not in table                           # terminated process
