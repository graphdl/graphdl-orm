"""FORML derivation-storage markers (NORMA */**/+/++): a fact type reading carrying a trailing
marker links the fact type to its derivation method and storage method, tagged in M."""
from pyarest import forml, from_lam
import pyarest.prims  # noqa: F401


def cells(D, name):
    for c in from_lam(D):
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_strip_derivation_markers():
    assert forml._strip_derivation("F *") == ("fully-derived", "F")
    assert forml._strip_derivation("F **") == ("derived-and-stored", "F")
    assert forml._strip_derivation("F +") == ("semi-derived", "F")
    assert forml._strip_derivation("F ++") == ("partially-derived-and-stored", "F")
    assert forml._strip_derivation("F") == (None, "F")


def test_derived_fact_type_tagged_in_M():
    model = "Student is an entity type.\nCourse is an entity type.\nStudent audits Course *."
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    assert ("Student_audits_Course", "fully-derived") in cells(D, "derivation")
    assert ("Student_audits_Course", "{0} audits {1}") in cells(D, "factType")   # the reading is still declared


def test_derived_and_stored_and_semiderived():
    D, _ = forml.compile_model("Person is an entity type.\n"
                               "Person is grandparent of Person **.\n"
                               "Person mentors Person +.")
    kinds = {k for (_ft, k) in cells(D, "derivation")}
    assert {"derived-and-stored", "semi-derived"} <= kinds
