"""Grammar extensions from the verbalization paper: mixfix predicate templates (front text,
n-ary, hyphen binding), negative forms (mapping to the same constraint as their positive twin),
and inclusive-or / disjunctive mandatory. Dispatch is table-driven (no if/elif)."""
from pyarest import forml
from pyarest.lam import from_lam
import pyarest.prims  # noqa: F401


def cells(D, name):
    for c in from_lam(D):
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


# ---- mixfix predicate templates (field replacement, {i} placeholders) ----
def test_reading_templates():
    k = ["Person", "Country", "Date"]
    assert forml._reading("Person was born in Country", k) == ("{0} was born in {1}", ["Person", "Country"])
    assert forml._reading("the birth of Person occurred in Country", k) == ("the birth of {0} occurred in {1}", ["Person", "Country"])
    assert forml._reading("Person introduced Person to Person on Date", k) == ("{0} introduced {1} to {2} on {3}", ["Person", "Person", "Person", "Date"])


def test_front_text_fact_type_is_stored_as_template():
    model = ("Person is an entity type.\nCountry is an entity type.\n"
             "the birth of Person occurred in Country.")
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    assert any(reading == "the birth of {0} occurred in {1}" for (_id, reading) in cells(D, "factType"))


# ---- negative forms map to the same constraint as the positive twin ----
def test_negative_uniqueness_equals_positive():
    Dn, _ = forml.compile_model("It is impossible that any Person was born in more than one Country.")
    Dp, _ = forml.compile_model("Each Person was born in at most one Country.")
    cid = "Person_was_born_in_Country_uc"
    assert cid in {c[0] for c in cells(Dn, "constraint")}
    assert cid in {c[0] for c in cells(Dp, "constraint")}


def test_negative_mandatory_classified():
    # the NORMA neg-mandatory spelling has no corpus demand and no grammar
    # wiring yet: the pin after the seed deletion is honest REFUSAL (the
    # statement reports, nothing junk declares) until a corpus asks for it
    D, rep = forml.compile_model(
        "It is impossible that any Person was born in no Country.")
    flagged = rep["unparsed"] + rep.get("prose", [])
    assert len(flagged) == 1
    assert not [f[0] for f in cells(D, "factType")
                if "impossible" in str(f[0]).lower()]


# ---- inclusive-or / disjunctive mandatory ----
def test_inclusive_or():
    D, _ = forml.compile_model("Each Vehicle was purchased from some Retailer or is rented.")
    assert any(c[1] == "disjunctive_mandatory" for c in cells(D, "constraint"))


# ---- dispatch is a table, not if/elif ----
def test_plan_dispatch_is_a_table():
    assert isinstance(forml._PLAN, dict) and "uniqueness" in forml._PLAN
