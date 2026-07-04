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
    assert len(rep["unparsed"]) == 1
    nouns = {r[0] for r in system._pop_rows(D, "instanceOf")}
    assert not {"MCP", "Task Readiness", "Frontier", "Familiar Name"} & nouns
