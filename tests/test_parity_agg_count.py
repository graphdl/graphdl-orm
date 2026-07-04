"""The first real-app parity fixture: apps/agg-count-check, vendored verbatim. The
app was the old engine's own repro for a defect its ledger records (count aggregates
returned EMPTY with a check warning on the deployed binary while min populated), and
the steward's recorded truth was Pair_has_tally = {p01:2, p02:1, p12:1}. pyarest
must compile the app's exact readings — markdown headers, the leading * derivation
markers, the iff spelling, where-scoped aggregate bodies, unnumbered aggregate
outputs, and the count op — and produce the steward's numbers. Fix-not-inherit made
concrete: the count populates here."""
import pyarest.prims  # noqa: F401
from pyarest.lam import from_lam
from pyarest import forml, system

READINGS = """# Agg Count Check — does `count` aggregate populate where `min` does?

## Entity Types

Pair(.id) is an entity type.

Count(.id) is an entity type.

Tally(.id) is an entity type.

## Fact Types

Pair has Count.

Pair has cheapest Count. *

Pair has Tally. *

## Derivation Rules

* Pair1 has cheapest Count iff Count is the min of Count2 where Pair1 has Count2.

* Pair1 has Tally iff Tally is the count of Count2 where Pair1 has Count2.

## Instance Facts

Pair 'p01' has Count '1'.
Pair 'p01' has Count '3'.
Pair 'p02' has Count '2'.
Pair 'p12' has Count '1'.
"""


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_the_real_app_compiles_to_the_stewards_numbers():
    D, rep = forml.compile_model(READINGS)
    assert rep["unparsed"] == []
    assert rep["rule_diagnostics"] == []
    D = system.run_rules(D)
    Dpy = from_lam(D)
    assert _cell(Dpy, "Pair_has_cheapest_Count") == \
        {("p01", "1"), ("p02", "2"), ("p12", "1")}
    assert _cell(Dpy, "Pair_has_Tally") == \
        {("p01", 2), ("p02", 1), ("p12", 1)}                  # line 2778, verbatim
