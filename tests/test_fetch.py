"""↑ fidelity (Backus §13.3.4): fetch of an absent cell is #, the default object — not φ.
The create pipeline's fresh-cell default (an absent cell reads as an empty population) is
an explicit choice in build_system, not a change to ↑'s meaning."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import ast
from pyarest.reduce import apply


def _D(*cells):
    l = L.NIL
    for c in reversed(cells):
        l = L.CONS(c)(l)
    return L.SEQ(l)


def test_fetch_absent_cell_is_the_default_object():
    D = _D(ast.cell("OTHER", to_lam((("x",),))))
    assert from_lam(apply(ast.Fetch("nope"), D)) == "#"          # ↑ of an absent cell = #
    assert from_lam(apply(ast.Fetch("OTHER"), D)) == (("x",),)   # present cell unchanged


def test_create_on_a_fresh_cell_still_defaults_to_empty_population():
    # the PIPELINE maps # -> φ, so a create against a store with no such cell commits into
    # a fresh cell (this is the forml ingestion path: new M cells appear on first assert)
    (o, Dp) = from_lam(ast.run(to_lam(("a", "x")), _D(ast.cell("OTHER", to_lam(())))))
    (p2, _v) = o
    assert ("a", "x") in p2
    file_cells = [c for c in Dp if isinstance(c, tuple) and c[:2] == ("CELL", "FILE")]
    assert file_cells and set(file_cells[0][2]) == {("a", "x")}
