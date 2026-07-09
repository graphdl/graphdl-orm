"""Transition identity rekey (engine.rekey_transitions): machine-scope each
transition to a surrogate 'txn:{SMD}\\x1f{name}' (Core.png/GraphDL: Transition(.id)
is a nameless surrogate) so base-vs-app reuse of a transition NAME does not merge
one entity across two machines. Per compile pass: bare names get a surrogate keyed
by their defined-in SMD; rows already surrogate-keyed (base's, in the app pass) are
skipped. Every Transition-typed position is rewritten — role metamodel + machinery
sm* + the non-role-typed Guard_prevents_Transition — so no reference dangles.

Gated on a synthetic per-pass sequence: without the rekey the two machines' 'review'
merge and sm_triples cross-products (2x2x2); with it they stay distinct (exactly the
2 real triples)."""
import pyarest.prims  # noqa: F401
from pyarest import engine
from pyarest.lam import from_lam, to_lam

ROLE = (("r1", "Transition_is_from_Status", 1, "Transition"),
        ("r2", "Transition_is_to_Status", 1, "Transition"),
        ("r3", "Transition_is_defined_in_State_Machine_Definition", 1, "Transition"),
        ("r4", "Guard_prevents_Transition", 2, "Transition"))


def D_of(cells):
    return to_lam(tuple(("CELL", n, tuple(rows)) for n, rows in cells.items()))


def cell(D, n):
    for c in from_lam(D):
        if isinstance(c, tuple) and c[0] == "CELL" and c[1] == n:
            return [tuple(r) for r in c[2]]
    return []


def _base_pass():
    base = D_of({
        "role": ROLE,
        "Transition_is_defined_in_State_Machine_Definition": (("review", "MachineA"),),
        "smFrom": (("review", "Proposed"),), "smTo": (("review", "Reviewed"),),
        "smTrigger": (("review", "evA"),),
        "Guard_prevents_Transition": (("g1", "review"),),
    })
    return engine.rekey_transitions(base)


def test_base_pass_surrogate_and_guard():
    b2 = _base_pass()
    assert cell(b2, "smFrom") == [("txn:MachineA\x1freview", "Proposed")]
    # the non-role-typed guard cell (transition at pos 1) is rewritten too
    assert cell(b2, "Guard_prevents_Transition") == [("g1", "txn:MachineA\x1freview")]


def test_app_pass_skips_surrogate_and_resolves_collision():
    b2 = _base_pass()
    app = D_of({
        "role": ROLE,
        "Transition_is_defined_in_State_Machine_Definition":
            tuple(cell(b2, "Transition_is_defined_in_State_Machine_Definition")) + (("review", "MachineB"),),
        "smFrom": tuple(cell(b2, "smFrom")) + (("review", "FineReceived"),),
        "smTo": tuple(cell(b2, "smTo")) + (("review", "UnderReview"),),
        "smTrigger": tuple(cell(b2, "smTrigger")) + (("review", "evB"),),
    })
    a2 = engine.rekey_transitions(app)
    sf = cell(a2, "smFrom")
    assert not [r for r in sf if r and r[0] == "review"]          # no bare name left
    assert len({r[0] for r in sf}) == 2                           # two distinct surrogates
    triples = {tuple(t) for t in engine.sm_triples(a2)}
    assert triples == {("Proposed", "evA", "Reviewed"),
                       ("FineReceived", "evB", "UnderReview")}    # no cross-product
