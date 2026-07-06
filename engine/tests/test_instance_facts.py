"""Instance facts, the corpus's dominant form (47% of readings statements): quoted ids
fill the roles of a declared fact type, and the row lands in that fact type's own cell,
which is the population the runtime reads. `Operation 'create' applies in View Context
'collection'.` populates Operation_applies_in_View_Context with ("create", "collection")."""
import pyarest.prims  # noqa: F401
from pyarest.lam import from_lam
from pyarest import forml


MODEL = """Operation is an entity type.
View Context is an entity type.
Operation applies in View Context.
Operation 'create' applies in View Context 'collection'.
Operation 'read' applies in View Context 'detail'.
Status is an entity type.
Status 'In Cart' exists.
"""


def _cell(D, name):
    for c in from_lam(D):
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_instance_facts_populate_the_fact_types_cell():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    pop = _cell(D, "Operation_applies_in_View_Context")
    assert ("create", "collection") in pop and ("read", "detail") in pop


def test_unary_instance_fact():
    D, _ = forml.compile_model(MODEL)
    assert ("In Cart",) in _cell(D, "Status_exists")


def test_quoted_ids_may_contain_spaces_and_do_not_break_the_template():
    D, _ = forml.compile_model(MODEL)
    fts = {f[0] for f in _cell(D, "factType")}
    assert "Operation_applies_in_View_Context" in fts          # declared once, cleanly
