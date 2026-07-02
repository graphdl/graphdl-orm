"""The corpus residue forms plus the bitemporal kernel. Brace subtype families
('{A, B} are mutually exclusive subtypes of X.', 9 corpus occurrences) assert the
subtype rows plus the pairwise exclusion; namespaced nouns (schema:Thing) already parse
and are pinned here. Bitemporal τ (Halpin §13.6): transaction time is when the system
records a fact, valid time is ordinary UoD data in the fact itself; create_stamped
records ⟨tx, …fact⟩ beside the base fact in the same step, and as_of reconstructs the
population at any past transaction time — Prop. onestep's order_τ audit view."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


BRACES = """Party is an entity type.
{Person, Company} are mutually exclusive subtypes of Party.
"""


def test_brace_subtype_family():
    D, rep = forml.compile_model(BRACES)
    assert rep["unparsed"] == []
    assert rep["kinds"].get("fact_type_reading", 0) == 0      # no junk fact type
    Dpy = from_lam(D)
    assert {("Person", "Party"), ("Company", "Party")} <= _cell(Dpy, "subtype")
    cons = _cell(Dpy, "constraint")
    assert any(f[1] == "exclusion" and set(f[3]) == {"Person", "Company"}
               for f in cons if len(f) >= 4)


def test_namespaced_nouns_parse_as_ordinary_nouns():
    D, rep = forml.compile_model("""schema:Person is an entity type.
schema:Thing is an entity type.
schema:Person knows schema:Thing.
""")
    assert rep["unparsed"] == []
    fts = {f[0] for f in _cell(from_lam(D), "factType")}
    assert "schema_Person_knows_schema_Thing" in fts


def test_transaction_time_is_recorded_and_as_of_reconstructs():
    D, _ = forml.compile_model("Person is an entity type.\nPerson knows Person.\n")
    ft = "Person_knows_Person"
    D = system.create_stamped(D, ft, to_lam(("a", "b")), tx=1)
    D = system.create_stamped(D, ft, to_lam(("b", "c")), tx=2)
    assert system.as_of(D, ft, 1) == {("a", "b")}             # the past, reconstructed
    assert system.as_of(D, ft, 2) == {("a", "b"), ("b", "c")}
    assert _cell(from_lam(D), ft) == {("a", "b"), ("b", "c")}  # the present, unchanged
