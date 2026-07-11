"""The mixfix reading scan as a canonical object (the keystone's first leaf
over the lex boundary). system:reading_parse — ⟨text, knowns, stop⟩ →
⟨template, roles⟩: the paper's field-replacement model, scanning left to
right over lex records, replacing each known type occurrence (LONGEST match,
maximal munch — order-independent, no pre-sorted operand) with a {i}
placeholder; NORMA hyphen binding (#24 — leading 'adj- {N}' and trailing
'{N} -adj' keep the WORD in the template with the one-sided touching hyphen
consumed, '--' escapes to one literal hyphen, and a hyphen touching both
sides ('from-Status') is just a word: the retired touching bind); the atomic
Title-case-run guard (a match whose continuation token is Title-case with no
known name covering the extended span is predicate text). system:ftid —
⟨template, roles⟩ → the stable fact-type id (roles substituted back, slugged).
The Python _reading/_ftid_from are the behavioral spec and become thin
callers; the whole shared/base corpus is the twin oracle."""
import os

import pyarest.prims  # noqa: F401
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest.reduce import apply

STOP = ("If", "When", "Then", "That", "This", "An", "A", "The", "Each",
        "Some", "No", "Every", "Not", "It", "There", "Once", "For", "In",
        "Of", "To", "On", "At", "By", "With", "And", "Or", "Only")


def _parse(text, knowns):
    return from_lam(apply(A("system:reading_parse"),
                          to_lam((text, tuple(knowns), STOP))))


def test_binary_reading_with_trailing_and_front_text():
    assert _parse("Order was placed by Customer", ("Order", "Customer")) == \
        ("{0} was placed by {1}", ("Order", "Customer"))
    assert _parse("the birth of Person occurred in Country",
                  ("Person", "Country")) == \
        ("the birth of {0} occurred in {1}", ("Person", "Country"))


def test_unary_and_multiword_types_take_the_longest_match():
    assert _parse("Person smokes", ("Person",)) == ("{0} smokes", ("Person",))
    # maximal munch: 'Event Type' wins over 'Event'
    assert _parse("Event Type has Name", ("Event", "Event Type", "Name")) == \
        ("{0} has {1}", ("Event Type", "Name"))


def test_the_atomic_title_case_run_guard():
    # 'Layer' inside 'has Layer Affinity to' is predicate text: the run
    # 'Layer Affinity' is uncorroborated by any known name
    assert _parse("Thing has Layer Affinity to Thing2",
                  ("Thing", "Layer")) == \
        ("{0} has Layer Affinity to Thing2", ("Thing",))
    # but the extended span being KNOWN corroborates it
    assert _parse("Thing has Layer Affinity", ("Thing", "Layer",
                                               "Layer Affinity")) == \
        ("{0} has {1}", ("Thing", "Layer Affinity"))


def test_norma_hyphen_binding_leading_trailing_escape_and_touching():
    """The four NORMA forms (#24), certified host == canon per form: the bind
    marker is consumed but the WORD stays (so 'Transition is from- Status' and
    'Transition is from Status' answer the SAME template — the respell is a
    store no-op and the ftid is invariant); '--' keeps a literal hyphen; the
    old touching bind is retired (just a word — no role claimed)."""
    from pyarest import forml
    ks = ("Layer", "Coord")
    cases = [
        ("Layer has valence- Coord", ("{0} has valence {1}", ("Layer", "Coord"))),
        ("Layer has Coord -local", ("{0} has {1} local", ("Layer", "Coord"))),
        ("Layer has valence-- Coord", ("{0} has valence- {1}", ("Layer", "Coord"))),
        ("Layer has --local Coord", ("{0} has -local {1}", ("Layer", "Coord"))),
        ("Layer has valence-Coord", ("{0} has valence-Coord", ("Layer",))),
    ]
    for text, want in cases:
        assert _parse(text, ks) == want, text                 # the canon
        t, r = forml._reading(text, set(ks))
        assert (t, tuple(r)) == want, text                    # the host twin


def test_the_bind_respelling_leaves_the_ftid_invariant():
    # the SM readings' identity: 'Transition is from- Status' collapses to the
    # very ftid the plain spelling mints (spec #24, the Transition example)
    ks = ("Transition", "Status")
    plain = _parse("Transition is from Status", ks)
    bound = _parse("Transition is from- Status", ks)
    assert plain == bound == ("{0} is from {1}", ("Transition", "Status"))
    got = from_lam(apply(A("system:ftid"), to_lam(bound)))
    assert got == "Transition_is_from_Status"


def test_ftid_substitutes_back_and_slugs():
    got = from_lam(apply(A("system:ftid"),
                         to_lam(("{0} was placed by {1}",
                                 ("Order", "Customer")))))
    assert got == "Order_was_placed_by_Customer"
    got2 = from_lam(apply(A("system:ftid"),
                          to_lam(("{0} has {1}", ("Layer", "Coord")))))
    assert got2 == "Layer_has_Coord"


def test_the_canonical_scan_twins_the_python_reading_over_the_base_corpus():
    """The strongest oracle: every fact-type reading in shared/base answers the
    SAME (template, roles) through the canonical object as through _reading."""
    from pyarest import forml
    root = os.path.join(os.path.dirname(os.path.dirname(
        os.path.abspath(__file__))), "shared", "base")
    text = "\n\n".join(open(os.path.join(root, f), encoding="utf-8").read()
                       for f in sorted(os.listdir(root)) if f.endswith(".md"))
    stmts = forml.statements(text)
    known = set(forml._known(stmts))
    checked = 0
    for s in stmts:
        kind, g, _m = forml.analyze(s)
        if kind != "fact_type_reading" or "'" in g[0] \
                or forml._prose_suspect(g[0], known):
            continue
        reading = g[0]
        want = forml._reading(reading, known)
        got = _parse(reading, sorted(known))
        assert got == (want[0], tuple(want[1])), (reading, got, want)
        checked += 1
    assert checked >= 100                                     # the corpus is real
