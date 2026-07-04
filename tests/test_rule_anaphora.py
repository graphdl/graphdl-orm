"""The NORMA-style anaphoric derivation rule form (the live corpus's spelling,
canonical per the old grammar's own classifier: a statement with the keyword iff
IS a Derivation Rule — arest readings/forml2-grammar.md — with no numbered-
variable requirement). Variables are type-name occurrences; the qualifiers
that/some/the/other/a/an strip away (the old engine's strip_role_qualifiers);
numeric subscripts distinguish same-type variables; a quoted literal after a
role mention restricts that role; a bare 'A is B' clause over two known types
with no declared fact type is a COERCION binding (the tasks app's re-keying
idiom), aliasing the two variables to one column."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system


SM_MODEL = """Status is a value type.
Resource is an entity type.
State Machine is an entity type.
Task is an entity type.
Task Status is a value type.
State Machine is for Resource.
State Machine is currently in Status.
Task has Task Status.

* Resource is currently in Status iff some State Machine is for that Resource and that State Machine is currently in that Status.

* Task has Task Status iff that Resource is currently in some Status and Task Status is Status and Task is Resource.

State Machine 'sm1' is for Resource 't1'.
State Machine 'sm1' is currently in Status 'in_progress'.
State Machine 'sm2' is for Resource 't2'.
"""


def _derive(model):
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    return system.run_rules(D), rep


def test_anaphoric_rule_derives_through_that_and_some():
    D, rep = _derive(SM_MODEL)
    got = {tuple(r) for r in system._pop_rows(D, "Resource_is_currently_in_Status")}
    # sm2 declares no current status: t2 has no derived row (no cascade, no junk)
    assert got == {("t1", "in_progress")}


def test_coercion_clauses_rekey_the_derived_fact():
    D, rep = _derive(SM_MODEL)
    got = {tuple(r) for r in system._pop_rows(D, "Task_has_Task_Status")}
    assert got == {("t1", "in_progress")}


def test_iff_statements_never_become_fact_types():
    D, rep = _derive(SM_MODEL)
    fts = [f[0] for f in system._pop_rows(D, "factType")]
    assert not [ft for ft in fts if "_iff_" in ft], fts
    assert rep["rule_diagnostics"] == []


LIT_MODEL = """Task is an entity type.
Task Status is a value type.
Task Priority is a value type.
Task has Task Status.
Task has Task Priority.
Task Priority is recommended.

* Task is recommended iff Task has Task Status 'in_progress' and Task has Task Priority and Task Priority is recommended.

Task 't1' has Task Status 'in_progress'.
Task 't1' has Task Priority 'p0'.
Task 't2' has Task Status 'open'.
Task 't2' has Task Priority 'p0'.
Task 't3' has Task Status 'in_progress'.
Task 't3' has Task Priority 'p9'.
Task Priority 'p0' is recommended.
"""


def test_in_body_literal_restricts_the_role():
    D, rep = _derive(LIT_MODEL)
    got = {tuple(r) for r in system._pop_rows(D, "Task_is_recommended")}
    # t2 fails the literal ('open'), t3's priority is not recommended
    assert got == {("t1",)}
    assert rep["rule_diagnostics"] == []


def test_numbered_rules_still_compile_the_same():
    model = """Person is an entity type.
Person likes Person.
Person admires Person.

Person1 admires Person2 iff Person1 likes Person2.

Person 'a' likes Person 'b'.
"""
    D, rep = _derive(model)
    got = {tuple(r) for r in system._pop_rows(D, "Person_admires_Person")}
    assert got == {("a", "b")}
