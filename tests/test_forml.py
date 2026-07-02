"""Widened FORML grammar (NORMA verbalization) → M-facts + compiled constraints, via
compile∘parse = create over M. compile_model folds a whole verbalization into M."""
import pytest
from pyarest import from_lam, to_lam, apply
from pyarest.lam import atom as A
import pyarest.prims  # noqa: F401
from pyarest import forml, meta
from pyarest import constraints as C  # noqa: F401


def cells(D, name):
    for c in from_lam(D):
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


MODEL = """
Person is an entity type.
Country is an entity type.
Person is identified by PersonName.
Each Person was born in at most one Country.
It is possible that some Person visited some Country.
Each Employee is an instance of Person.
The possible values of Rating are 1, 2, 3.
No Person parents itself.
"""


def test_parse_recognizes_norma_families():
    assert forml.parse("Person is an entity type.")[0] == "entity_type"
    assert forml.parse("PersonName is a value type.")[0] == "value_type"
    assert forml.parse("Person is identified by PersonName.")[0] == "ref_scheme"
    assert forml.parse("Each Person was born in at most one Country.")[0] == "binary"
    assert forml.parse("It is possible that some Person visited some Country.")[0] == "possibility"
    assert forml.parse("Each Employee is an instance of Person.")[0] == "subtype"
    assert forml.parse("The possible values of Rating are 1, 2, 3.")[0] == "value_constraint"
    assert forml.parse("No Person parents itself.")[0] == "ring_irreflexive"


def test_compile_model_ingests_a_verbalization():
    D = forml.compile_model(MODEL)
    io = cells(D, "instanceOf")
    assert {("Person", "ObjectType"), ("Country", "ObjectType"),
            ("PersonName", "ValueType"), ("Employee", "ObjectType")} <= io
    assert ("Employee", "Person") in cells(D, "subtype")
    assert ("Person", "PersonName") in cells(D, "refScheme")
    assert ("Person_was_born_in_Country", "was born in") in cells(D, "factType")
    assert ("Person_was_born_in_Country_uc", "uniqueness", "Person_was_born_in_Country") in cells(D, "constraint")
    assert ("Rating", ("1", "2", "3")) in cells(D, "valueConstraint")
    assert ("Person_parents_Person_irreflexive", "ring_irreflexive", "Person_parents_Person") in cells(D, "constraint")


def test_compiled_constraint_reflects_and_enforces():
    forml.compile_model(MODEL)                               # defines the constraint objects in DEFS
    pop = to_lam((("alice", "france"), ("alice", "spain")))  # alice born in two countries
    v = from_lam(apply(A("Person_was_born_in_Country_uc"), pop))
    assert set(v) == {("alice", "france"), ("alice", "spain")}   # uniqueness reflected, (ρc):P = V_c


def test_mandatory_from_some_quantifier():
    D = forml.compile_model("Each Person has some Address.")
    assert ("Person_has_Address_mand", "mandatory", "Person_has_Address") in cells(D, "constraint")


def test_out_of_R_rejected():
    with pytest.raises(ValueError):
        forml.parse("Person who drives Car")


def test_nf_idempotent():
    for r in ("Person is an entity type", "Name is a value type"):
        assert forml.nf(r) == r
