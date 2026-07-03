"""The conceptual schema optimizer, first increment: decisions as derived facts.

Halpin's optimization procedure (book 12.5) leaves four factors to judgment: target
system, query pattern, update pattern, clarity. Here they are data. The TRIGGERS are
constraint patterns M already holds (an exclusive unary family for step 4; a small
enumerated role inside an n-ary for PSG1 absorption under the table width guideline).
The THRESHOLDS are declared facts (optThreshold rows; the default is Halpin's own
"reasonable number (e.g., 5)"). The QUERY PATTERN is a measured read log, host-side
like the event log (Prop. onestep: the log's order "is no fact of the domain"), so
"focused" is a count, not a vibe. plan(D) is a pure analysis returning suggestions
with their grounds; the apply side comes later behind the population oracle, and the
authored M is never rewritten (Halpin sanctions the transforms as an invisible
preprocessing stage to Rmap)."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam
from pyarest import ast, forml, system, optimize
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


MODEL = """Employee(.nr) is an entity type.
Employee is salaried.
Employee is hourly.
Employee is casual.
For each Employee, at most one of the following holds: that Employee is salaried; that Employee is hourly; that Employee is casual.
Country(.code) is an entity type.
MedalKind is a value type.
The possible values of MedalKind are 'gold', 'silver', 'bronze'.
Tally is a value type.
Country won Tally of MedalKind.
"""


def test_exclusive_unary_family_suggests_generalization():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    sugg = [s for s in optimize.plan(D) if s["kind"] == "generalize_exclusive_unaries"]
    assert len(sugg) == 1
    s = sugg[0]
    assert s["noun"] == "Employee"
    assert set(s["fact_types"]) == {"Employee_is_salaried", "Employee_is_hourly",
                                    "Employee_is_casual"}
    assert s["grounds"]["constraint"] == "Employee_excl"      # the firing fact, cited


def test_small_enumerated_role_in_nary_suggests_absorption():
    D, _ = forml.compile_model(MODEL)
    sugg = [s for s in optimize.plan(D) if s["kind"] == "absorb_enumerated_role"]
    assert len(sugg) == 1
    s = sugg[0]
    assert s["fact_type"] == "Country_won_Tally_of_MedalKind"
    assert s["role"] == 3 and s["value_type"] == "MedalKind"
    assert s["grounds"]["width"] == 3                         # ≤ the declared threshold


def test_thresholds_are_declared_facts_not_code_constants():
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.Store("optThreshold"), S(to_lam((("enum_width", 2),)), D))
    kinds = {s["kind"] for s in optimize.plan(D)}
    assert "absorb_enumerated_role" not in kinds              # width 3 > declared 2
    assert "generalize_exclusive_unaries" in kinds            # unaffected family


def test_the_read_log_measures_focus():
    D, _ = forml.compile_model(MODEL)
    optimize.reset_read_log()
    for _ in range(3):
        rows = optimize.read_pop(D, "Country_won_Tally_of_MedalKind")
    assert rows == set()                                      # empty population, read fine
    assert optimize.read_counts() == {"Country_won_Tally_of_MedalKind": 3}
    plans = optimize.plan(D, reads=optimize.read_counts())
    assert plans[0]["kind"] == "absorb_enumerated_role"       # the hot ft ranks first
    assert plans[0]["reads"] == 3
    optimize.reset_read_log()
    assert optimize.read_counts() == {}


def test_a_model_without_triggers_yields_no_suggestions():
    D, _ = forml.compile_model("Person(.id) is an entity type.\n"
                               "Name is a value type.\nPerson has Name.\n")
    assert optimize.plan(D) == []
