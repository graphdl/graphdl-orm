"""Finality as a schema declaration (writer model item 2): the depth k at which
optimistic acceptance hardens from deontic to alethic is a per-noun declaration in M
('X becomes final at depth N.'); Nakamoto §11's table makes any chosen k quantitative.
finality_modality is the hardening rule the writer runtime binds: below k a violation
reports deontically (accept + flag + repair obligation), at or beyond k it refuses
alethically. An undeclared noun is final immediately (the safe default)."""
import pyarest.prims  # noqa: F401
from pyarest.lam import from_lam
from pyarest import forml, system


MODEL = """Order(.OrderId) is an entity type.
Order becomes final at depth 6.
"""


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_finality_declares_into_M():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    assert ("Order", 6) in _cell(from_lam(D), "finality")


def test_the_hardening_rule():
    D, _ = forml.compile_model(MODEL)
    assert system.finality_modality(D, "Order", 0) == "deontic"
    assert system.finality_modality(D, "Order", 5) == "deontic"
    assert system.finality_modality(D, "Order", 6) == "alethic"
    assert system.finality_modality(D, "Undeclared", 0) == "alethic"


def test_nf_is_idempotent_on_finality():
    assert forml.nf("Order becomes final at depth 6.") == \
        forml.nf(forml.nf("Order becomes final at depth 6."))
