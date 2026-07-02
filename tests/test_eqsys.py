"""eq. sys: the whole system as values in D into one lambda. SYSTEM fetches the addressed
entity's handler FROM D and applies it; an unaddressable entity reduces to bottom."""
from pyarest import from_lam, to_lam
from pyarest.lam import atom as A
import pyarest.lam as L
import pyarest.prims  # noqa: F401
from pyarest import ast


def _D(*cells):
    l = L.NIL
    for c in reversed(cells):
        l = L.CONS(c)(l)
    return L.SEQ(l)

def _cell_named(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return c[2]
    return None


def test_eq_sys_routes_to_the_entity_handler_in_D():
    # D holds the entity's handler as a value (a cell) and its data cell; SYSTEM routes to it.
    handler = ast.build_system(cell_name="people")            # create over the "people" data cell
    D = _D(ast.cell("addPerson", handler), ast.cell("people", to_lam(())))
    (o, Dp) = from_lam(ast.dispatch("addPerson", to_lam(("Alice", "30")), D))
    (p2, _v) = o
    assert ("Alice", "30") in p2                              # the handler ran
    assert ("Alice", "30") in _cell_named(Dp, "people")       # committed to the entity's data cell

def test_eq_sys_tenant_unaddressability():
    # an address naming no cell of D fetches # ; #:x reduces to ⊥ — wrong-tenant access is
    # not forbidden but impossible (Prop. tenant)
    handler = ast.build_system(cell_name="people")
    D = _D(ast.cell("addPerson", handler), ast.cell("people", to_lam(())))
    assert from_lam(ast.dispatch("ghost", to_lam(("x", "y")), D)) == "⊥"
