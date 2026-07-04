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


def test_a_literal_containing_iff_stays_an_instance_fact():
    # the claude app's Operating Rule corpus: a rule-statement LITERAL quoting the
    # word iff must not route the instance fact into the rule path (the old
    # engine's keyword scan is literal-aware)
    model = """Operating Rule is an entity type.
Rule Statement is a value type.
Operating Rule has Rule Statement.

Operating Rule 'r1' has Rule Statement 'A thing is an entity iff you can SEE it'.
"""
    D, rep = _derive(model)
    got = {tuple(r) for r in system._pop_rows(D, "Operating_Rule_has_Rule_Statement")}
    assert got == {("r1", "A thing is an entity iff you can SEE it")}
    assert rep["rule_diagnostics"] == []


def test_a_literal_containing_digits_and_iff_stays_an_instance_fact():
    # the rule_if (numbered) recognizer has the same blindness when the only
    # digit sits inside the literal (claude's Engine Lesson corpus)
    model = """Engine Lesson is an entity type.
Construction is a value type.
Engine Lesson prescribes Construction.

Engine Lesson 'e1' prescribes Construction 'Layer has Load 0 iff Layer stacks'.
"""
    D, rep = _derive(model)
    got = {tuple(r) for r in system._pop_rows(D, "Engine_Lesson_prescribes_Construction")}
    assert got == {("e1", "Layer has Load 0 iff Layer stacks")}
    assert rep["rule_diagnostics"] == []


def test_a_head_literal_projects_as_a_constant():
    # the claude app's deontic-flag idiom: the head fixes one role to a literal,
    # projected as a constant (rho interprets a ⟨CONST, lit⟩ spec entry)
    model = """Investigation is an entity type.
Hypothesis is an entity type.
Reasoning Practice is a value type.
Hypothesis Disposition is a value type.
Investigation should apply Reasoning Practice.
Hypothesis belongs to Investigation.
Hypothesis has Hypothesis Disposition.

* Investigation should apply Reasoning Practice 'Systematic Debugging' iff some Hypothesis belongs to that Investigation and that Hypothesis has Hypothesis Disposition 'disproven'.

Hypothesis 'h1' belongs to Investigation 'i1'.
Hypothesis 'h1' has Hypothesis Disposition 'disproven'.
Hypothesis 'h2' belongs to Investigation 'i2'.
"""
    D, rep = _derive(model)
    got = {tuple(r) for r in system._pop_rows(
        D, "Investigation_should_apply_Reasoning_Practice")}
    assert got == {("i1", "Systematic Debugging")}
    assert rep["rule_diagnostics"] == []


def test_unnumbered_aggregate_clause_compiles():
    # the base's own spelling: '* Fact Type has Arity iff Arity is the count of
    # Role where Fact Type has Role.' — no numbered variables anywhere
    model = """Fact Type is an entity type.
Role is an entity type.
Arity is a value type.
Fact Type has Role.
Fact Type has Arity.

* Fact Type has Arity iff Arity is the count of Role where Fact Type has Role.

Fact Type 'f1' has Role 'r1'.
Fact Type 'f1' has Role 'r2'.
Fact Type 'f2' has Role 'r3'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(r) for r in system._pop_rows(D, "Fact_Type_has_Arity")}
    assert got == {("f1", 2), ("f2", 1)}


def test_the_readings_trailing_marker_owns_the_storage_kind():
    # the rule's leading star marks the RULE; the fact type's storage kind
    # comes from its READING declaration ('X. **' = derived-and-stored, plain =
    # no derivation mark at all even when a rule also derives into it — the old
    # base's SM current-status has two imperative writers beside its seed rule)
    model = """Person is an entity type.
Login is an entity type.
Status is a value type.
Person has Status.
Login has Status.
Login is for Person.
Task is an entity type.
Task has Status. **

* Person has Status iff some Login is for that Person and that Login has Status.
"""
    D, rep = forml.compile_model(model)
    der = {r[0]: r[1] for r in system._pop_rows(D, "derivation") if len(r) >= 2}
    assert "Person_has_Status" not in der                     # declared plain: no mark
    assert der.get("Task_has_Status") == "derived-and-stored"  # the reading's marker
    rd = {r[1] for r in system._pop_rows(D, "ruleDerives") if len(r) >= 2}
    assert "Person_has_Status" in rd                          # the rule still derives


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
