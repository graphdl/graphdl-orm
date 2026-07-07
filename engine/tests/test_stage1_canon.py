"""Stage-1 tokenization gets a CANONICAL INTERFACE (the L1 arc's last
pocket): the stage1_fields PRIM at the lex boundary — ⟨text, vocab, nouns, sid⟩ → the statement's
field-fact rows ⟨⟨field ft, ⟨sid, value⟩⟩…⟩, the rows classify_all_via_M
asserts before the recognizer rules derive classifications. Per Samuel
(2026-07-07, operating rule regex-impl-in-defs-ok): the IMPLEMENTATION may
be host regex registered against the DEF name when performant and PROVEN
TO THE CORRECT INTERFACE — these contract tests are that proof's frame,
their expectations traced from tokenize_statement (compiler.py:1001), the
behavioral spec. The vocabulary arrives AS AN OPERAND (hoisted once per
sweep — the compile hot pocket's fix), never recomputed per statement."""
import pyarest.prims  # noqa: F401
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest.reduce import apply

VOCAB = (("Statement_has_Copula", "is a"),
         ("Statement_has_Entity_Marker", "is an entity type"),
         ("Statement_has_Trailing_Marker", "is a value type"))


def _fields(text, vocab, nouns, sid):
    got = from_lam(apply(A("stage1_fields"),
                         to_lam((text, tuple(vocab), tuple(nouns), sid))))
    assert isinstance(got, tuple), got          # ⊥ = the def is missing
    return got


def test_vocabulary_hits_report_field_facts():
    got = _fields("Person is an entity type.", VOCAB, ("Person",), "s1")
    assert ("Statement_has_Entity_Marker",
            ("s1", "is an entity type")) in got
    assert ("Statement_has_Role_Reference", ("s1", "Person")) in got


def test_a_trailing_marker_must_trail():
    got = _fields("Name is a value type of Person.", VOCAB, ("Person",), "s2")
    # 'is a value type' occurs but does NOT trail: no Trailing Marker fact
    assert not any(f[0] == "Statement_has_Trailing_Marker" for f in got)
    trail = _fields("Name is a value type.", VOCAB, (), "s3")
    assert ("Statement_has_Trailing_Marker",
            ("s3", "is a value type")) in trail


def test_quoted_literals_blind_the_recognizers_and_report_the_role():
    got = _fields("Task 't1' has Status 'is a'.", VOCAB, ("Task",), "s4")
    # the token inside the quoted literal must NOT fire the vocabulary
    assert not any(f[0] == "Statement_has_Copula" for f in got)
    assert ("Statement_has_Literal_Role", ("s4", "t1")) in got


def test_prose_punctuation_reports_once_outside_literals():
    got = _fields("Send it, then wait.", (), (), "s5")
    assert ("Statement_has_Prose_Punctuation", ("s5", ",")) in got
    quoted = _fields("Task 'a, b' has Status.", (), (), "s6")
    assert not any(f[0] == "Statement_has_Prose_Punctuation" for f in quoted)
