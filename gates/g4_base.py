"""G4 — the base satisfies its own schema (SPEC 10.2, §13 G4).

The metamodel is an app of the system (Cor 5): compile the base readings and
run the absolute sweep; zero alethic violations or the base does not ship.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

_BASE = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                     "readings", "base")

# core first (the vocabulary the rest builds on), then the alphabetical rest
_ORDER = ["core.md"] + sorted(f for f in os.listdir(_BASE)
                              if f.endswith(".md") and f != "core.md")


def _base_D():
    from host_py import forml
    text = "\n\n".join(open(os.path.join(_BASE, f), encoding="utf-8").read()
                       for f in _ORDER)
    return forml.compile_model(text)


def test_base_compiles_whole():
    D, rep = _base_D()
    assert not rep.get("unparsed"), rep.get("unparsed")


def test_base_satisfies_its_own_schema():
    from host_py import gate
    D, _ = _base_D()
    bad = gate.alethic(gate.sweep(D))
    assert bad == [], "\n".join(
        f"{v['fact_type']} {v['kinds']}: {v['offenders'][:5]}{'…' if len(v['offenders']) > 5 else ''}"
        for v in bad)
