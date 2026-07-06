"""The nf round trip (Prop. spec; conformance gate 1): nf = verbalize ∘ compile ∘ parse is
idempotent, and nf(r) is a sentence of the fragment that re-parses to the same construct.
Metamorphic — no golden outputs: for every kind the grammar accepts, nf(nf(r)) == nf(r),
the kind survives, and the modality survives. Cross-form normalization (a negative twin
verbalizing back positive) is the kernel quotient ~ and is tested by compile equivalence
in test_grammar; here each kind keeps its own canonical surface."""
import pyarest.prims  # noqa: F401
from pyarest import forml


BATTERY = [
    "Order(.OrderId) is an entity type.",
    "Name is a value type.",
    "Reference Scheme: Person has Name.",
    "Reference Mode: nr.",
    "Data Type: string.",
    "The possible values of Grade are A, B, C.",
    "In each population of Order includes Product, each Order, Product combination occurs at most once.",
    "In each population of Expert is on Panel, each Panel combination occurs at least 2 times.",
    "Person is parent of Person is acyclic.",
    "Woman is a subtype of Patient.",
    "This association with Enrollment provides the preferred identification scheme for Enrollment.",
    "For each Message, at most one of the following holds: that Message is with some Phone; that Message is with some Email.",
    "Each Vehicle was purchased from some Retailer or is rented.",
    "If some Message matches some Rep then that Message is sent by that Rep.",
    "Message is with Phone if and only if Rep has Phone.",
    "*Each FastCarDriver is some Person who drives some Car that is fast.",
    "Person ~smokes.",
    "Each Order is placed by exactly one Customer.",
    "Each Student has some Email.",
    "Customer places Order.",
    "State Machine Definition 'Order' is for Noun 'Order'.",
    "Status 'In Cart' is initial in State Machine Definition 'Order'.",
    "Transition 'place' is from Status 'In Cart'.",
    "Transition 'place' is to Status 'Placed'.",
    "Transition 'place' is triggered by Fact Type 'Customer places Order'.",
    "It is obligatory that each Student has at most one Email.",
    "It is forbidden that each Person was born in more than one Country.",
    "It is possible that more than one Student has the same Email.",
]


def test_nf_is_idempotent_across_the_battery():
    for r in BATTERY:
        once = forml.nf(r)
        assert forml.nf(once) == once, r


def test_nf_preserves_kind_and_modality():
    for r in BATTERY:
        k0, _g0, m0 = forml.analyze(r)
        k1, _g1, m1 = forml.analyze(forml.nf(r))
        assert (k1, m1) == (k0, m0), (r, k0, k1, m0, m1)


def test_nf_of_a_constraint_compiles_to_the_same_object():
    r = "It is obligatory that each Student has at most one Email."
    Da, _ = forml.compile_model(r)
    Db, _ = forml.compile_model(forml.nf(r))
    from pyarest.lam import from_lam
    cells_a = {c[1]: set(c[2]) for c in from_lam(Da) if isinstance(c, tuple) and c[0] == "CELL"}
    cells_b = {c[1]: set(c[2]) for c in from_lam(Db) if isinstance(c, tuple) and c[0] == "CELL"}
    assert cells_a.get("constraint") == cells_b.get("constraint")


def test_nf_rejects_outside_the_fragment():
    # arbitrary period-terminated prose IS a fact-type reading in this fragment (the
    # catch-all recognizer), so the genuinely-outside case is the empty statement
    import pytest
    with pytest.raises(ValueError):
        forml.nf("")
