"""Three corpus spellings the base and live apps use, surfaced by the prose
guard: a trailing parenthetical is an ANNOTATION stripped from the reading (the
old cell is Verb_is_performed_during_Transition, no Mealy suffix); the for-each
mandatory ('For each Reading, some Role is used in that Reading.') declares the
fact type through the anaphoric scan and mandates the for-each subject at its
role position; and the corpus spanning-uniqueness spelling ('Each API, Noun
combination occurs at most once in the population of <reading>.') is the
in-population form with the roles in front."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system


def test_a_trailing_parenthetical_is_an_annotation_not_reading_text():
    model = """Verb is an entity type.
Transition is an entity type.
Verb is performed during Transition (Mealy semantics).
Verb 'ship' is performed during Transition 't1'.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    got = {tuple(r) for r in system._pop_rows(D, "Verb_is_performed_during_Transition")}
    assert got == {("ship", "t1")}


def test_for_each_mandatory_declares_and_mandates():
    model = """Reading is an entity type.
Role is an entity type.
Role is used in Reading.
For each Reading, some Role is used in that Reading.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    fts = {f[0] for f in system._pop_rows(D, "factType")}
    assert "Role_is_used_in_Reading" in fts
    rows = [tuple(c) for c in system._pop_rows(D, "constraint")
            if len(c) >= 4 and c[1] == "mandatory"]
    assert ("Role_is_used_in_Reading_mand", "mandatory",
            "Role_is_used_in_Reading", "Reading", "alethic") in rows


def test_corpus_spanning_uniqueness_spelling():
    model = """API is an entity type.
Noun is an entity type.
API accepts Noun as parameter.
Each API, Noun combination occurs at most once in the population of API accepts Noun as parameter.
API 'a1' accepts Noun 'n1' as parameter.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    rows = [tuple(c) for c in system._pop_rows(D, "constraint")
            if len(c) >= 3 and c[1] in ("uniqueness", "spanning_uniqueness")]
    assert any(c[2] == "API_accepts_Noun_as_parameter" for c in rows)
    got = {tuple(r) for r in system._pop_rows(D, "API_accepts_Noun_as_parameter")}
    assert got == {("a1", "n1")}
