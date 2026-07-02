"""Definitions are ordinary cells of D (Backus §13.3.5): a cell ⟨CELL, n, c⟩ has the same
effect as Def n ≡ ρc, in the same namespace as data cells ("some cells may name data
rather than functions", §14.3). DEFS names the whole-state accessor: defs:x = D (§14.3.3).
"""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, defs
from pyarest import reduce as R
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _D(*cells):
    l = L.NIL
    for c in reversed(cells):
        l = L.CONS(c)(l)
    return L.SEQ(l)


BASE = lambda: _D(ast.cell("FILE", to_lam(())))


def test_a_definition_is_an_ordinary_cell_of_D_under_its_own_name():
    D = apply(ast.DefineIn("shout3", S(A("CONST"), A("LOUD"))), BASE())
    cells = [c for c in from_lam(D) if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"]
    assert any(c[1] == "shout3" for c in cells)               # the definition IS a cell
    assert not any(c[1] == "DEFS" for c in cells)             # no separate DEFS container
    (o, _Dp) = from_lam(ast.run(to_lam(("f",)), D, derive_obj=A("shout3")))
    assert o[0] == "LOUD"                                     # and it resolves in the step


def test_DEFS_is_the_whole_state_accessor():
    D = BASE()
    with defs.step(D):
        assert from_lam(R.apply(A("DEFS"), to_lam("x"))) == from_lam(D)
        assert from_lam(R.apply_lambda(A("DEFS"), to_lam("x"))) == from_lam(D)


def test_data_cells_share_the_namespace_but_are_not_functions():
    # applying a DATA name metacomposes into the population and bottoms; fetching is the
    # intended use of a data name (§14.3) and the step's transition rule reports the error
    D = _D(ast.cell("FILE", to_lam((("a", "b"),))))
    with defs.step(D):
        assert from_lam(R.apply(A("FILE"), to_lam("x"))) == "⊥"
