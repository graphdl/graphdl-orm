"""Verbatim Backus fidelity, from reading the primary source in full (2026-07-02).

§13.3.4: ↓n is (push n)∘[1, (pop n)∘2] — it removes the FIRST cell named n and prepends
the new one; deeper same-named cells are PRESERVED ("multiple, named, LIFO stacks within
a storage sequence"). pop and purge are distinct operators Backus defines side by side.

§14.3.1: "If μ(SYSTEM:x) is not a pair, the output is an error message and the state
remains unchanged." The transition rules (element 3 of §14.3) are the AST framework's,
outside the applicative subsystem — so run/dispatch enforce them.
"""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast
from pyarest.reduce import apply


def _D(*cells):
    l = L.NIL
    for c in reversed(cells):
        l = L.CONS(c)(l)
    return L.SEQ(l)


def test_store_is_push_pop_not_purge():
    # two cells named X (a LIFO stack); ↓X replaces only the TOP; the deeper X survives
    D = _D(ast.cell("X", to_lam((1,))), ast.cell("X", to_lam((2,))), ast.cell("Y", to_lam((3,))))
    D2 = apply(ast.Store("X"), L.SEQ(L.CONS(to_lam((9,)))(L.CONS(D)(L.NIL))))
    cells = [c for c in from_lam(D2) if isinstance(c, tuple) and c[0] == "CELL"]
    xs = [c[2] for c in cells if c[1] == "X"]
    assert xs == [(9,), (2,)]                                 # top replaced, stack preserved
    assert [c[2] for c in cells if c[1] == "Y"] == [(3,)]


def test_fetch_still_reads_the_top_of_the_stack():
    D = _D(ast.cell("X", to_lam((1,))), ast.cell("X", to_lam((2,))))
    assert from_lam(apply(ast.Fetch("X"), D)) == (1,)         # first match wins (§13.3.4)


def test_non_pair_system_result_yields_error_and_unchanged_state():
    # an unaddressable entity reduces to ⊥ — not a pair — so the TRANSITION RULE answers:
    # error output, state unchanged (§14.3.1), rather than a bare ⊥ escaping the step
    handler = ast.build_system(cell_name="people")
    D = _D(ast.cell("addPerson", handler), ast.cell("people", to_lam(())))
    (o, Dp) = from_lam(ast.dispatch("ghost", to_lam(("x",)), D))
    assert o == "ERROR"                                       # the error message output
    assert Dp == from_lam(D)                                  # and D is unchanged


def test_run_with_a_bottom_stage_also_reports_error():
    D = _D(ast.cell("FILE", to_lam(())))
    (o, Dp) = from_lam(ast.run(to_lam(("a",)), D, derive_obj=A("no-such-def")))
    assert o == "ERROR" and Dp == from_lam(D)
