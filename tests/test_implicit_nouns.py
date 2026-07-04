"""Implicit role nouns, the old corpus's style: a maximal run of Title-case
tokens inside a reading IS a role noun even when no declaration names it (the
old engine's Role Reference extraction; its tasks db binds Event Type, Fact
Type, Noun this way). Resolution stays longest-first, so an implicit 'Event
Type' beats a declared 'Event' — the migration rehearsal caught event-type
populations landing in the event table without this."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system


def test_an_implicit_multiword_noun_beats_a_declared_prefix():
    model = """Event is an entity type.
Transition is an entity type.
Transition is triggered by Event Type.
Transition 't1' is triggered by Event Type 'created'.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    roles = {}
    for r in system._pop_rows(D, "role"):
        if len(r) >= 4:
            roles.setdefault(r[1], []).append((r[2], r[3]))
    assert sorted(roles["Transition_is_triggered_by_Event_Type"]) == [
        (1, "Transition"), (2, "Event Type")]
    got = {tuple(r) for r in system._pop_rows(
        D, "Transition_is_triggered_by_Event_Type")}
    assert got == {("t1", "created")}


def test_predicate_text_is_not_a_noun_without_instance_corroboration():
    # the claude verdict's phantom role: 'Layer Affinity' in 'has Layer
    # Affinity to' is PREDICATE TEXT (never instance-quoted anywhere), so the
    # fact type has TWO roles and the migrated two-wide rows join; 'Engineering
    # Lever' and 'Layer' are corroborated by their quoted instances
    model = """Engineering Lever is an entity type.
Layer is an entity type.
Engineering Lever has Layer Affinity to Layer.

Engineering Lever 'e1' has Layer Affinity to Layer 'SPD1-4'.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    roles = {}
    for r in system._pop_rows(D, "role"):
        if len(r) >= 4:
            roles.setdefault(r[1], []).append((r[2], r[3]))
    assert sorted(roles["Engineering_Lever_has_Layer_Affinity_to_Layer"]) == [
        (1, "Engineering Lever"), (2, "Layer")]
    got = {tuple(r) for r in system._pop_rows(
        D, "Engineering_Lever_has_Layer_Affinity_to_Layer")}
    assert got == {("e1", "SPD1-4")}


def test_quantifier_position_corroborates_a_noun():
    # the base's own shape: 'Fact Type' is never declared and never
    # instance-quoted, but 'Each Fact Type has exactly one Arity.' quantifies
    # over it — a quantifier position names a type by construction (Halpin's
    # constraint verbalizations), so the noun survives and the aggregate keeps
    # its grouping column (the verdict-five regression: 748 arities became 1)
    model = """Role is an entity type.
Arity is a value type.
Fact Type has Role.
Fact Type has Arity.
Each Fact Type has exactly one Arity.
"""
    stmts = forml.statements(model)
    names = set(forml._known(stmts))
    assert "Fact Type" in names                               # the quantifier names it
    t, roles = forml._reading("Fact Type has Arity", names)
    assert roles == ["Fact Type", "Arity"]                    # two roles, grouping intact


def test_frequency_phrases_do_not_corroborate_predicate_text():
    # solver-loop.md:53, the twelve-hypothesis hunt's answer: 'Each Engineering
    # Lever has at most one Layer Affinity to Layer.' must NOT re-noun 'Layer
    # Affinity' through the 'one' of the frequency phrase — the rule's clause
    # would parse three variables and project column 3 of two-wide join rows
    model = """Engineering Lever is an entity type.
Layer is an entity type.
Engineering Lever has Layer Affinity to Layer.
Each Engineering Lever has at most one Layer Affinity to Layer.

Engineering Lever 'e1' has Layer Affinity to Layer 'SPD1-4'.
"""
    stmts = forml.statements(model)
    names = set(forml._known(stmts))
    assert "Layer Affinity" not in names
    t, roles = forml._reading("Engineering Lever has Layer Affinity to Layer",
                              names)
    assert roles == ["Engineering Lever", "Layer"]


def test_implicit_nouns_do_not_mine_prose_or_literals():
    model = """Task is an entity type.
Rule Statement is a value type.
Task has Rule Statement.

Task 't1' has Rule Statement 'Match the Frontier, not the Familiar Name'.

Once the engine surfaces user-domain facts via MCP query (issue 821), the Task Readiness derivation fires.
"""
    D, rep = forml.compile_model(model)
    # the prose paragraph stays unparsed; no 'MCP' or 'Task Readiness' or
    # 'Frontier' noun appears from prose or quoted literals
    flagged = rep["unparsed"] + rep.get("prose", [])
    assert len(flagged) == 1
    nouns = {r[0] for r in system._pop_rows(D, "instanceOf")}
    assert not {"MCP", "Task Readiness", "Frontier", "Familiar Name"} & nouns
