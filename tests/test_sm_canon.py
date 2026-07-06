"""The state-machine translator's canonical MEANING object (whitepaper §1: a
machine is a SET OF FACTS in M). system:sm_rows is the α-shaped DEF carrying
which M-fact rows an sm statement asserts: ⟨verb, head, l1, l2⟩ → ⟨⟨cell, row⟩…⟩,
the verb being the grammar's own recognizer token (forml2-grammar.md), the head
splitting the one shared verb ('emits': Transition→Mealy, Status→Moore), and the
literals the statement's quoted operands (trigger/guard arrive RESOLVED — the
reading→fact-type-id step is the boundary's, not this object's). The Python
handlers are thin callers; this DEF is the meaning."""
import pyarest.prims  # noqa: F401
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest.reduce import apply


def _rows(verb, head, l1, l2):
    return from_lam(apply(A("system:sm_rows"), to_lam((verb, head, l1, l2))))


def test_sm_def_asserts_the_machinery_fact_and_the_instance_fact():
    assert _rows("is for Noun", "State Machine Definition", "Order", "Order") == (
        ("smDef", ("Order", "Order")),
        ("State_Machine_Definition_is_for_Noun", ("Order", "Order")))


def test_sm_initial_swaps_into_the_sm_status_triple():
    # Status 'In Cart' is initial in State Machine Definition 'Order'.
    assert _rows("is initial in State Machine Definition", "Status",
                 "In Cart", "Order") == (
        ("smStatus", ("Order", "In Cart", "initial")),      # ⟨sm, status, initial⟩
        ("Status_is_initial_in_State_Machine_Definition", ("In Cart", "Order")))


def test_sm_from_and_to_assert_both_cells():
    assert _rows("is from Status", "Transition", "place", "In Cart") == (
        ("smFrom", ("place", "In Cart")),
        ("Transition_is_from_Status", ("place", "In Cart")))
    assert _rows("is to Status", "Transition", "place", "Placed") == (
        ("smTo", ("place", "Placed")),
        ("Transition_is_to_Status", ("place", "Placed")))


def test_sm_trigger_and_guard_take_the_resolved_fact_type():
    assert _rows("is triggered by Fact Type", "Transition",
                 "place", "Customer_places_Order") == (
        ("smTrigger", ("place", "Customer_places_Order")),)
    assert _rows("is guarded by Fact Type", "Transition",
                 "place", "Order_is_paid") == (
        ("smGuard", ("place", "Order_is_paid")),)


def test_the_emits_verb_splits_on_the_head():
    assert _rows("emits", "Transition", "place", "place-receipt") == (
        ("smEmit", ("place", "place-receipt")),)
    assert _rows("emits", "Status", "Placed", "awaiting-shipment") == (
        ("smMoore", ("Placed", "awaiting-shipment")),)


def test_the_python_handlers_are_thin_callers_of_the_canonical_rows():
    """Twin oracle: for every sm statement form, the production path (_plan over
    the extracted groups) answers exactly the canonical object's rows."""
    from pyarest import forml
    CASES = [
        ("State Machine Definition 'Order' is for Noun 'Order'.", "sm_def"),
        ("Status 'In Cart' is initial in State Machine Definition 'Order'.",
         "sm_initial"),
        ("Transition 'place' is from Status 'In Cart'.", "sm_from"),
        ("Transition 'place' is to Status 'Placed'.", "sm_to"),
        ("Transition 'place' is triggered by Fact Type 'Customer places Order'.",
         "sm_trigger"),
        ("Transition 'place' is guarded by Fact Type 'Order includes Product'.",
         "sm_guard"),
        ("Transition 'place' emits 'place-receipt'.", "sm_emit"),
        ("Status 'Placed' emits 'awaiting-shipment'.", "sm_moore"),
    ]
    HEAD = {"sm_def": "State Machine Definition", "sm_initial": "Status",
            "sm_moore": "Status"}
    VERB = {"sm_def": "is for Noun",
            "sm_initial": "is initial in State Machine Definition",
            "sm_from": "is from Status", "sm_to": "is to Status",
            "sm_trigger": "is triggered by Fact Type",
            "sm_guard": "is guarded by Fact Type",
            "sm_emit": "emits", "sm_moore": "emits"}
    known = forml._Known({"Customer", "Order", "Product"}, {}, set(), set())
    for stmt, kind in CASES:
        got_kind, g, _m = forml.analyze(stmt)
        assert got_kind == kind
        facts, objs = forml._plan(kind, g, known, "alethic")
        assert objs == []
        l2 = (forml._clause_ft(g[1], known)
              if kind in ("sm_trigger", "sm_guard") else g[1])
        assert tuple(facts) == _rows(VERB[kind], HEAD.get(kind, "Transition"),
                                     g[0], l2)
