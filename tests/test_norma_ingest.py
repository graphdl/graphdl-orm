"""Drop-in of a NORMA verbalization: compile_model ingests a whole document into M (object
types, value types, objectified associations, fact types, constraints) and the compiled
constraints enforce (rho c):P = V_c. The fixture is a synthetic university model exercising
every grammar family (declarations, ref schemes, fact types, uniqueness/mandatory, the
possibility twin, spanning UC, objectification, set-comparison, subset (if..then), negation)."""
import os
from pyarest import forml
from pyarest.lam import from_lam, to_lam, atom
from pyarest.reduce import apply
import pyarest.prims  # noqa: F401

SAMPLE = os.path.join(os.path.dirname(__file__), "norma_sample.txt")


def _cells(D, name):
    for c in from_lam(D):
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_whole_verbalization_parses():
    _D, rep = forml.compile_model(open(SAMPLE, encoding="utf-8").read())
    assert rep["unparsed"] == []                              # every statement recognized
    assert rep["total"] >= 40


def test_schema_extracted_matches_the_model():
    D, _rep = forml.compile_model(open(SAMPLE, encoding="utf-8").read())
    ents = {t[0] for t in _cells(D, "instanceOf") if t[1] == "ObjectType"}
    vals = {t[0] for t in _cells(D, "instanceOf") if t[1] == "ValueType"}
    assert {"Student", "Course", "Instructor"} <= ents        # the entity types
    assert "Enrollment" in ents                               # objectified association
    assert {"Name", "Email", "Grade"} <= vals
    assert "Enrollment" in {t[0] for t in _cells(D, "objectification")}


def test_dropped_in_constraint_enforces():
    from pyarest import defs
    D, _rep = forml.compile_model(open(SAMPLE, encoding="utf-8").read())   # defines into D's DEFS
    pop = to_lam((("s1", "a@x"), ("s1", "b@x")))             # s1 has two Emails
    with defs.step(D):                                       # resolution is per-store (Cor. closure)
        v = from_lam(apply(atom("Student_has_Email_uc"), pop))
    assert set(v) == {("s1", "a@x"), ("s1", "b@x")}          # (rho c):P = V_c, from the parsed schema
