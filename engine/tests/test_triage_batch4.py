"""Fourth triage batch: the subset-check ordering seam (evaluate.rs's
test_subset_violation / test_subset_constraint_without_autofill_produces_violation).

Def. create validates the candidate post-state, and a derived fact type's population
is DEFINED by the closure (Def. derive: lfp of F_S), so a subset constraint whose
consequent is filled from its antecedent by a COPY rule can never be violated in the
post-state: the rule IS the proof of inclusion, and the check is statically
DISCHARGED. Without such a rule the subset check gates writes as before (the old
engine's without-autofill violation). The same discharge covers the subtype family:
_h_subtype installs the inclusion rule super(x) <- sub(x) alongside its subset check,
so writing a subtype instance is not refused for the supertype row the rule itself
would derive. M records the copy shape as ruleCopies facts (single positive atom,
no filters, identity head projection) and validate_for consults them at assembly
time, where M is already load-bearing."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import forml, defs
from pyarest.reduce import apply


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def _check(val, pop, D):
    with defs.step(D):
        _p, v, flag = from_lam(apply(val, L.SEQ(L.CONS(to_lam(pop))(L.CONS(D)(L.NIL)))))
    return set(v), flag


BARE = """Academic(.id) is an entity type.
Department(.id) is an entity type.
Academic heads Department.
Academic works for Department.
If some Academic heads some Department then that Academic works for that Department.
"""


def test_bare_subset_gates_the_unmatched_antecedent_write():
    D, rep = forml.compile_model(BARE)
    assert rep["unparsed"] == []
    val = forml.validate_for("Academic_heads_Department", D)
    v, flag = _check(val, (("A1", "D1"),), D)                 # works-for is empty
    assert v == {("A1", "D1")} and flag == "T"                # alethic: blocks commit


def test_subset_discharged_by_the_filling_copy_rule():
    MODEL = BARE + """Academic works for Department +.
Academic1 works for Department1 if Academic1 heads Department1.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    assert rep["rule_diagnostics"] == []
    val = forml.validate_for("Academic_heads_Department", D)
    v, flag = _check(val, (("A1", "D1"),), D)
    # the copy rule proves heads ⊆ works at every fixed point: no violation, no block
    assert v == set() and flag == "F"


def test_subtype_membership_write_is_not_blocked_by_its_own_inclusion():
    MODEL = """Vehicle(.id) is an entity type.
Car is an entity type.
Car is a subtype of Vehicle.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    val = forml.validate_for("Car", D)
    v, flag = _check(val, (("c1",),), D)                      # Vehicle cell still empty
    # the installed inclusion rule Vehicle(x) <- Car(x) discharges the subtype check
    assert v == set() and flag == "F"


def test_a_permuted_head_is_not_a_copy_and_keeps_the_check():
    MODEL = """Person(.id) is an entity type.
Person likes Person.
Person admires Person.
If some Person likes some Person then that Person admires that Person.
Person1 admires Person2 if Person2 likes Person1.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    assert rep["rule_diagnostics"] == []
    val = forml.validate_for("Person_likes_Person", D)
    v, flag = _check(val, (("p1", "p2"),), D)                 # admires is empty
    # the rule SWAPS the roles: likes(p1,p2) derives admires(p2,p1), which does NOT
    # witness likes ⊆ admires — the check must survive
    assert v == {("p1", "p2")} and flag == "T"
