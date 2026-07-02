"""The FORML grammar over NORMA's actual verbalization forms: classify recognizes each
family; compile_model folds a document into M and returns (D, coverage report). Examples use
a synthetic university domain."""
from pyarest import forml
from pyarest.lam import from_lam
import pyarest.prims  # noqa: F401


def cells(D, name):
    for c in from_lam(D):
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def k(s):
    return forml.classify(s)[0]


def test_classify_norma_families():
    assert k("Student is an entity type.") == "entity_type"
    assert k("Grade is a value type.") == "value_type"
    assert k("Reference Scheme: Student has Student_id.") == "ref_scheme"
    assert k("Data Type: Text: Variable Length (0).") == "data_type"
    assert k("Each Student has at most one Email.") == "uniqueness"
    assert k("Each Course is taught by exactly one Instructor.") == "uniqueness"
    assert k("Each Student has some Name.") == "mandatory"
    assert k("It is possible that more than one Student has the same Email.") == "possibility"
    assert k("In each population of Student enrolls in Course, each Student, Course combination occurs at most once.") == "spanning_uc"
    assert k("This association with Student, Course provides the preferred identification scheme for Enrollment.") == "objectification"
    assert k("For each Enrollment, at most one of the following holds: that Enrollment is by online; that Enrollment is by in-person.") == "set_comparison"
    assert k("If some Student enrolls in some Course then that Student is advised by some Instructor.") == "subset"
    assert k("Instructor ~is retired.") == "negation"
    assert k("Student has Name.") == "fact_type_reading"


def test_compile_model_small_returns_D_and_report():
    model = "Student is an entity type.\nEmail is a value type.\nEach Student has at most one Email."
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    assert ("Student", "ObjectType") in cells(D, "instanceOf")
    assert ("Email", "ValueType") in cells(D, "instanceOf")
    assert any(c[1] == "uniqueness" for c in cells(D, "constraint"))


def test_nf_idempotent():
    for r in ("Person is an entity type", "Name is a value type"):
        assert forml.nf(r) == r
