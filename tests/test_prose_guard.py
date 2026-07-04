"""The prose guard: a statement that would classify as a fact-type reading but
carries Title-case words resolving to NO declared type is PROSE — reported
unparsed, never silently declared (the old engine's #789 unresolved-Title-case
test, word-granular against the declared-name word set, with the 12-word Prose
Stopword vocabulary and subscript/plural normalization). The live corpus's
readings prose paragraphs (42 in the claude app) were becoming role-less junk
fact types; visibility is the fix, and the word-set leniency keeps legitimate
readings whose template words overlap declared multi-word types."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system


def test_a_prose_paragraph_is_unparsed_not_a_fact_type():
    model = """Task is an entity type.
Task Subject is a value type.
Task has Task Subject.

Once the engine surfaces user-domain facts via MCP query (issue 821), the Task Readiness derivation fires after every Task mutation.
"""
    D, rep = forml.compile_model(model)
    assert len(rep["unparsed"]) == 1
    assert "Once the engine" in rep["unparsed"][0]
    fts = [f[0] for f in system._pop_rows(D, "factType")]
    assert not [ft for ft in fts if "Once" in ft or "MCP" in ft]


def test_template_words_of_declared_multiword_types_stay_legitimate():
    # 'Target' appears in no standalone type, but 'Target SHA' is declared: the
    # word-granular set keeps the reading (the merge app's shape)
    model = """Merge is an entity type.
Target SHA is a value type.
Merge has Target SHA.
Each Merge has at most one Target SHA.
Merge 'm1' has Target SHA 'abc123'.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    assert {tuple(r) for r in system._pop_rows(D, "Merge_has_Target_SHA")} == {
        ("m1", "abc123")}


def test_stopwords_and_subscripts_do_not_flag():
    model = """Person is an entity type.
Person likes Person.
Each Person likes some Person.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []


def test_prose_containing_iff_stays_prose():
    # a documentation paragraph whose text contains ' iff ' must not become a
    # diagnosed rule: a real rule HEAD is a reading and never carries commas,
    # colons or parentheses (the claude app's two residual diagnostics)
    model = """Task is an entity type.
Hypothesis is an entity type.

The surface trigger is EXISTENTIAL: once any Hypothesis is disproven, the substrate flags iff the Task warrants it.
"""
    D, rep = forml.compile_model(model)
    assert len(rep["unparsed"]) == 1
    assert rep["rule_diagnostics"] == []
    assert not [f[0] for f in system._pop_rows(D, "factType")
                if "surface" in f[0].lower()]


def test_quoted_literals_are_not_scanned_for_prose():
    model = """Operating Rule is an entity type.
Rule Statement is a value type.
Operating Rule has Rule Statement.

Operating Rule 'r1' has Rule Statement 'Match the Frontier, not the Familiar Name'.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    assert ("r1", "Match the Frontier, not the Familiar Name") in {
        tuple(r) for r in system._pop_rows(D, "Operating_Rule_has_Rule_Statement")}
