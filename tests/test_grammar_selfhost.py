"""Grammar self-host, gate one (pyarest/readings/forml2-grammar.md, vendored: 'The
parser is not a program. It is this file.'): the grammar file ingests through
compile_model; its iff recognizers ('Statement has Classification C iff Statement has
Field ⟨literal⟩') compile into ordinary rules run by run_rules; and the Stage-1
tokenizer's ENTIRE vocabulary is read off the ingested file — the literals the rules
themselves test — so classification runs through the substrate. The gate:
classification-via-M agrees with the regex classifier across the kinds the file covers.
The regex seed remains as the bootstrap kernel (Stage-1's field extraction), which is
its designed role; the translators migrate next."""
import os
import pyarest.prims  # noqa: F401
from pyarest import forml

GRAMMAR_PATH = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                            "pyarest", "readings", "forml2-grammar.md")
_CACHE = {}


def _grammar_D():
    if "D" not in _CACHE:
        text = open(GRAMMAR_PATH, encoding="utf-8").read()
        _CACHE["D"] = forml.compile_model(text)
    return _CACHE["D"]


def test_the_grammar_file_ingests_with_its_rules_compiled():
    D, rep = _grammar_D()
    from pyarest.lam import from_lam
    Dpy = from_lam(D)
    rules = set()
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", "ruleDerives"):
            rules = {r[1] for r in c[2]}
    assert "Statement_has_Classification" in rules             # recognizers ARE rules


BATTERY = [
    ("Person is an entity type.", "Entity Type Declaration", ()),
    ("Rating is a value type.", "Value Type Declaration", ()),
    ("Person is a subtype of Party.", "Subtype Declaration", ()),
    ("Each Person holds at most one Passport.", "Uniqueness Constraint", ("Person", "Passport")),
    ("Each Order is placed by some Customer.", "Mandatory Role Constraint", ("Order", "Customer")),
    ("Person is a parent of Person. Person is a parent of Person is acyclic.".split(". ")[1],
     "Ring Constraint", ("Person",)),
    ("The possible values of Rating are '1', '2', '3'.", "Enum Values Declaration", ("Rating",)),
    ("Customer places Order.", "Fact Type Reading", ("Customer", "Order")),
    ("Customer 'c1' places Order 'o1'.", "Instance Fact", ("Customer", "Order")),
]


def test_classification_via_M_agrees_with_the_regex_classifier():
    D, _ = _grammar_D()
    for stmt, expected, nouns in BATTERY:
        got = forml.classify_via_M(D, stmt, nouns=nouns)
        assert expected in got, f"{stmt!r}: expected {expected} in {got}"


def test_the_tokenizer_vocabulary_comes_from_the_ingested_file():
    D, _ = _grammar_D()
    vocab = forml.stage1_vocabulary(D)
    assert ("Statement_has_Trailing_Marker", "is an entity type") in vocab
    assert ("Statement_has_Quantifier", "at most one") in vocab
    assert ("Statement_has_Keyword", "iff") in vocab           # read from D, not hardcoded
