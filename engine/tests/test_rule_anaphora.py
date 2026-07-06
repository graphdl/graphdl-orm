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


def test_sum_aggregate_with_mixed_numbering_and_lexical_values():
    # the claude corpus's spelling (compile-perf.md): unnumbered out-variable,
    # numbered source, 'total' as reading text — and the store carries LEXICAL
    # values ('120'), so the sum must coerce numeric atoms (the old engine's
    # values are conceptually typed; max matched while sum starved on this)
    model = """Compile Run is an entity type.
Compile Phase is an entity type.
Duration Ms is a value type.
Compile Run spends Duration Ms in Compile Phase.
Compile Run has total Duration Ms.

* Compile Run1 has total Duration Ms iff Duration Ms is the sum of Duration Ms1 where Compile Run1 spends Duration Ms1 in Compile Phase1.

Compile Run 'r1' spends Duration Ms '120' in Compile Phase 'parse'.
Compile Run 'r1' spends Duration Ms '30' in Compile Phase 'derive'.
Compile Run 'r2' spends Duration Ms '5' in Compile Phase 'parse'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(str(x) for x in r)
           for r in system._pop_rows(D, "Compile_Run_has_total_Duration_Ms")}
    assert got == {("r1", "150"), ("r2", "5")}


def test_mixed_type_rows_in_one_cell_do_not_crash_the_derive():
    # the migration seam: stored rows are LEXICAL ('150'), the coerced sum
    # derives ints (150) into the same head — the union must sort type-safely
    # (the claude rehearsal crashed here), and both rows coexist under NATEQ
    from pyarest import ast
    from pyarest.lam import to_lam
    from pyarest.reduce import apply as ap
    import pyarest.lam as L
    model = """Compile Run is an entity type.
Compile Phase is an entity type.
Duration Ms is a value type.
Compile Run spends Duration Ms in Compile Phase.
Compile Run has total Duration Ms.

* Compile Run1 has total Duration Ms iff Duration Ms is the sum of Duration Ms1 where Compile Run1 spends Duration Ms1 in Compile Phase1.

Compile Run 'r1' spends Duration Ms '120' in Compile Phase 'parse'.
"""
    D, rep = forml.compile_model(model)
    # a MIGRATED lexical total lands beside what the rule will derive
    pair = L.SEQ(L.CONS(to_lam((("r1", "120"),)))(L.CONS(D)(L.NIL)))
    D = ap(ast.Store("Compile_Run_has_total_Duration_Ms"), pair)
    D = system.run_rules(D)                                   # must not crash
    got = {tuple(str(x) for x in r)
           for r in system._pop_rows(D, "Compile_Run_has_total_Duration_Ms")}
    assert ("r1", "120") in got


def test_a_rule_survives_its_source_becoming_a_stored_cell():
    # the claude hunt's answer (seven falsified hypotheses deep): the rule ran
    # while its DERIVED source cell was absent and bottomed once run_rules
    # STORED it — the one-namespace step resolution served the CELL ROWS where
    # the fetch machinery meets the name. Storing the source by hand must not
    # change the rule's meaning.
    from pyarest import ast, defs
    from pyarest.lam import to_lam, atom as A, from_lam
    from pyarest.reduce import apply as ap
    import pyarest.lam as L
    model = """Engineering Lever is an entity type.
Layer is an entity type.
Engineering Lever is actionable.
Engineering Lever works Layer.
Layer is operator-loaded by Engineering Lever.

* Layer1 is operator-loaded by Engineering Lever1 iff Engineering Lever1 is actionable and Engineering Lever1 works Layer1.

Engineering Lever 'e1' works Layer 'L4'.
"""
    D, rep = forml.compile_model(model)
    # the source cell lands the way run_rules lands one: an explicit Store
    pair = L.SEQ(L.CONS(to_lam((("e1",),)))(L.CONS(D)(L.NIL)))
    D = ap(ast.Store("Engineering_Lever_is_actionable"), pair)
    rid = [r[0] for r in system._pop_rows(D, "ruleDerives")
           if len(r) >= 2 and r[1] == "Layer_is_operator_loaded_by_Engineering_Lever"][0]
    with defs.step(D):
        out = from_lam(ap(A(rid), D))
    assert isinstance(out, tuple), f"rule bottomed: {out!r}"
    assert ("L4", "e1") in {tuple(r) for r in out}


def test_a_unary_first_atom_joins():
    # the claude probe's bottom: a UNARY first atom followed by a join (every
    # prior rule test led with a binary) — 'EL is actionable and EL has
    # affinity to Layer' must derive, not bottom
    model = """Engineering Lever is an entity type.
Layer is an entity type.
Engineering Lever is actionable.
Engineering Lever works Layer.
Layer is operator-loaded by Engineering Lever.

* Layer1 is operator-loaded by Engineering Lever1 iff Engineering Lever1 is actionable and Engineering Lever1 works Layer1.

Engineering Lever 'e1' is actionable.
Engineering Lever 'e1' works Layer 'L4'.
Engineering Lever 'e2' works Layer 'L5'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(r) for r in system._pop_rows(
        D, "Layer_is_operator_loaded_by_Engineering_Lever")}
    # e1 is actionable and works L4; e2 is not actionable
    assert got == {("L4", "e1")}


def test_a_unary_first_join_through_a_predicate_text_reading():
    # variant two of the claude bottom: the joined reading carries an
    # uncorroborated Title-case run as PREDICATE TEXT ('has Layer Affinity to')
    model = """Engineering Lever is an entity type.
Layer is an entity type.
Engineering Lever is actionable.
Engineering Lever has Layer Affinity to Layer.
Layer is operator-loaded by Engineering Lever.

* Layer1 is operator-loaded by Engineering Lever1 iff Engineering Lever1 is actionable and Engineering Lever1 has Layer Affinity to Layer1.

Engineering Lever 'e1' is actionable.
Engineering Lever 'e1' has Layer Affinity to Layer 'L4'.
Engineering Lever 'e2' has Layer Affinity to Layer 'L5'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(r) for r in system._pop_rows(
        D, "Layer_is_operator_loaded_by_Engineering_Lever")}
    assert got == {("L4", "e1")}


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
