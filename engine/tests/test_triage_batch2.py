"""Second triage batch from the old engine's inline suites. rmap.rs: a PARTITIONED
subtype family (mutually exclusive — Halpin's partition mapping) keeps its own tables,
while a plain subtype absorbs to the top supertype as before; the SEMANTIC subtyping
(inclusion rules, clause lift) is unchanged — only the layout splits. check.rs: rules
that fail to compile must SAY WHY (the diagnostics class): a disconnected clause
variable or an unbound head variable surfaces in the report instead of vanishing into
M-facts-only silence. hateoas.rs is HTTP envelope surface — platform binding, out of
scope by the triage rules."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import from_lam
from pyarest import forml, system


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_partitioned_subtype_families_keep_their_own_tables():
    MODEL = """Party is an entity type.
Person is an entity type.
Company is an entity type.
Employee is an entity type.
{Person, Company} are mutually exclusive subtypes of Party.
Employee is a subtype of Person.
Name is a value type.
Person has Name.
Each Person has at most one Name.
Wage is a value type.
Employee has Wage.
Each Employee has at most one Wage.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    part = system.rmap_partition(D)
    # the partition boundary: Person keeps its own table (mutually exclusive family)
    assert part["Person_has_Name"] == "Person"
    # a PLAIN subtype still absorbs up to its nearest partitioned ancestor
    assert part["Employee_has_Wage"] == "Person"


def test_plain_subtypes_still_absorb_to_the_top():
    MODEL = """Party is an entity type.
Person is an entity type.
Person is a subtype of Party.
Name is a value type.
Person has Name.
Each Person has at most one Name.
"""
    D, _ = forml.compile_model(MODEL)
    part = system.rmap_partition(D)
    assert part["Person_has_Name"] == "Party"                 # step 0, unchanged


def test_a_rule_that_cannot_compile_says_why():
    # The unbound-head diagnostic CLASS retired 2026-07-08 (the skolem
    # compiler surface: an unbound head variable is its own skolem
    # function of the body frontier — the old positive form now
    # COMPILES). The says-why pin keeps its intent on a rule that still
    # cannot compile: a body with no fact-type clause at all.
    MODEL = """Person is an entity type.
Glyph is an entity type.
Person mentors Person.
Glyph links Glyph.
Person1 is odd if no Glyph2 links Glyph3.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    diags = _cell(from_lam(D), "ruleDiag")
    assert any("no fact-type clause" in reason for (_rid, reason) in diags)
    assert rep.get("rule_diagnostics")                            # surfaced in the report


def test_a_clean_rule_reports_no_diagnostics():
    MODEL = """Person is an entity type.
Person mentors Person.
Person1 is senior if Person1 mentors Person2.
"""
    D, rep = forml.compile_model(MODEL)
    assert _cell(from_lam(D), "ruleDiag") == set()
    assert not rep.get("rule_diagnostics")
