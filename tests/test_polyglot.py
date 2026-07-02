"""The polyglot differential: the SAME values — base-op probes, θ₁ joins, compiled
constraints, and the whitepaper machine's whole create handler — reduce under the
Python Scott mu and the Rust Scott mu (rust/src/main.rs, closures and Y all the way
down), and must agree exactly, bottoms included. No machine code exists in Rust: the
machine that advances there is the exported M-driven FFP object. Skipped cleanly when
the Rust binary has not been built (rust/ + cargo build --release)."""
import pytest
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, system, theta as T
from pyarest import polyglot

pytestmark = pytest.mark.skipif(not polyglot.rust_available(),
                                reason="rust kernel not built (cd rust; cargo build --release)")


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _diff(D, cases):
    got = polyglot.run_rust(polyglot.export_scenario(D, cases))
    want = polyglot.python_ground_truth(D, cases)
    for (case, g, w) in zip(cases, got, want):
        assert g == w, f"kernel divergence on {from_lam(case[0])!r}: rust={g!r} python={w!r}"


def test_base_ops_agree_across_kernels():
    D = L.SEQ(L.NIL)
    pop = to_lam((("a", 1), ("b", 2), ("a", 3)))
    comp = S(A("COMP"), A(1), A("tl"))
    consx = S(A("CONS"), A(2), A(1), S(A("CONST"), A("k")))
    cond = S(A("COND"), A("null"), S(A("CONST"), A("empty")), A("length"))
    ins = S(A("INSERT"), A("+"))
    alpha = S(A("ALPHA"), A(2))
    spin = S(A("WHILE"), S(A("CONST"), A("T")), A("id"))
    countdown = S(A("WHILE"),
                  S(A("COMP"), A("gt"), S(A("CONS"), A("id"), S(A("CONST"), A(0)))),
                  S(A("COMP"), A("-"), S(A("CONS"), A("id"), S(A("CONST"), A(1)))))
    cases = [
        (A(2), to_lam(("x", "y", "z")), None),
        (A(5), to_lam(("x", "y")), None),                     # short: ⊥ both sides
        (A("tl"), to_lam(("x",)), None),
        (A("tl"), A("atomic"), None),                         # ⊥
        (A("eq"), to_lam((1, 1.0)), None),                    # int vs float: F (NATEQ)
        (A("eq"), to_lam((1, 1)), None),
        (A("+"), to_lam((2, 3)), None),                       # int stays int
        (A("+"), to_lam((2, 1.5)), None),                     # promotes to float
        (A("div"), to_lam((4, 2)), None),                     # Python /: float
        (A("div"), to_lam((4, 0)), None),                     # ÷0 = ⊥
        (A("lt"), to_lam(("apple", "pear")), None),           # string ordering
        (A("trans"), to_lam(((1, 2), (3, 4), (5, 6))), None),
        (A("trans"), to_lam(((1, 2), (3,))), None),           # ragged: ⊥
        (A("apndl"), to_lam(("h", ("a", "b"))), None),
        (A("distr"), to_lam((("a", "b"), "y")), None),
        (A("rotr"), to_lam((1, 2, 3)), None),
        (A("1r"), to_lam((1, 2, 3)), None),
        (A("tlr"), to_lam(()), None),                         # tlr:φ = ⊥
        (comp, to_lam(("a", "b", "c")), None),
        (consx, to_lam(("p", "q")), None),
        (cond, to_lam(()), None),
        (cond, to_lam((1, 2, 3)), None),
        (ins, to_lam((1, 2, 3, 4)), None),
        (alpha, pop, None),
        (A("apply"), S(A(1), to_lam(("only",))), None),       # dynamic selection
        (spin, A(7), 3000),                                   # fuel bottoms both kernels
        (countdown, L.atom(5), 200000),                       # terminating WHILE agrees
    ]
    _diff(D, cases)


def test_theta_and_constraints_agree_across_kernels():
    from pyarest import constraints as C
    D = L.SEQ(L.NIL)
    pop = to_lam((("o1", "c1"), ("o2", "c2"), ("o1", "c3")))
    other = to_lam((("o1", "x"), ("o3", "y")))
    cases = [
        (T.Project([2, 1]), pop, None),
        (T.Filter(S(A("COMP"), A("eq"), S(A("CONS"), A(1), S(A("CONST"), A("o1"))))), pop, None),
        (T.NatJoin(1), S(pop, other), None),
        (C.uniqueness([1]), pop, None),                       # the UC violation expression
        (T.member, S(A("o2"), to_lam(("o1", "o2"))), None),
    ]
    _diff(D, cases)


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


def test_the_machine_advances_identically_on_rust_closures():
    # the killer case: the whitepaper create handler — commit, machine advance, links —
    # is ONE exported FFP object; Rust runs it with zero machine code of its own
    from pyarest.reduce import apply
    D, _ = forml.compile_model(ORDER)
    D = apply(ast.Store("Order_status"), S(to_lam((("o1", "In Cart"),)), D))
    trans_of = system.transitions_of(to_lam(system.sm_triples(D)), 2)
    handler = ast.build_system(
        cell_name="Customer_places_Order",
        machine=("Order_status", system.machine_step("Customer_places_Order"), 2),
        mealy_obj=system.mealy_step("Customer_places_Order"),
        links_obj=trans_of)
    x = S(to_lam(("c1", "o1")), D)
    _diff(D, [(handler, x, None)])


def test_rules_resolve_through_the_step_frame_on_both_kernels():
    SUPER = """Party is an entity type.
Person is an entity type.
Person is a subtype of Party.
State Machine Definition 'Party' is for Noun 'Party'.
"""
    D, _ = forml.compile_model(SUPER)
    D = system.governance_rules(D)
    # f is an ATOM: it must resolve through the step frame's DEFS in both kernels
    _diff(D, [(A("governedBy_rule_base"), D, None)])
