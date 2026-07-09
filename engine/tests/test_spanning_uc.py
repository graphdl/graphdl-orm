"""Composite (multi-column) uniqueness: the named columns RESOLVE
against the reading's roles, every name must land (a half-resolved
list silently compiled a TIGHTER constraint than declared — the
_components class, 2026-07-09: 'Property Name' missed the role named
'Property', the UC narrowed to Component alone, and every distinct row
flagged), and an unresolvable name reports as unparsed."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest import defs, forml, system
from pyarest.lam import to_lam, from_lam
from pyarest.reduce import apply as _ap


MODEL = """Component(.name) is an entity type.
Property Name is a value type.
Property Type is a value type.
Property Default is a value type.
Component has Property of Property Type with Property Default.
"""
FT = "Component_has_Property_of_Property_Type_with_Property_Default"
UC_OK = ("  Each Component, Property combination occurs at most once in the\n"
         "    population of Component has Property of Property Type"
         " with Property Default.\n")
UC_BAD = ("  Each Component, Property Name combination occurs at most once"
          " in the\n    population of Component has Property of Property"
          " Type with Property Default.\n")
ROWS = ("Component 'tab' has Property 'tabs' of Property Type 'string'"
        " with Property Default ''.\n"
        "Component 'tab' has Property 'selected' of Property Type 'int'"
        " with Property Default '0'.\n")


def _violations(model):
    D, rep = forml.compile_model(model)
    D = system.run_rules(D)
    part = system.rmap_partition(D)
    cand = tuple(tuple(r) for r in system._pop_rows(D, FT))
    val = forml.validate_for(FT, D, part)
    pair = L.SEQ(L.CONS(to_lam(cand))(L.CONS(D)(L.NIL)))
    with defs.step(D):
        _p, v, flag = from_lam(_ap(val, pair))
    return rep, v, flag


def test_distinct_rows_pass_the_resolved_composite():
    _rep, v, flag = _violations(MODEL + UC_OK + ROWS)
    assert flag == "F" and v == ()


def test_a_true_composite_duplicate_flags():
    dup = ROWS + ("Component 'tab' has Property 'tabs' of Property Type"
                  " 'bool' with Property Default 'x'.\n")
    _rep, v, flag = _violations(MODEL + UC_OK + dup)
    assert flag == "T" and len(v) == 2


def test_an_unresolvable_column_reports_and_never_narrows():
    rep, v, flag = _violations(MODEL + UC_BAD + ROWS)
    assert any("Property Name combination" in u
               for u in rep.get("unparsed") or [])
    assert flag == "F" and v == ()
