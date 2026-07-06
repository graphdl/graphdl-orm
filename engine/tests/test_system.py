"""The create pipeline over one cell: commit iff no alethic violation; derive = bounded lfp."""
from pyarest import apply, to_lam, from_lam
from pyarest.lam import atom as A
import pyarest.lam as L
import pyarest.prims  # noqa: F401
from pyarest import ast, system
from pyarest import constraints as C

def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)

def D_with(pop):
    return L.SEQ(L.CONS(ast.cell("FILE", to_lam(pop)))(L.NIL))

def file_of(Dpy):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL" and c[1] == "FILE":
            return set(c[2])
    return None


def test_create_commits_a_fact():
    (o, Dp) = from_lam(ast.run(to_lam(("a", "x")), D_with(())))
    (p2, v) = o
    assert ("a", "x") in p2 and v == ()
    assert file_of(Dp) == {("a", "x")}

def test_uniqueness_blocks_commit():
    val = system.validate_of([C.uniqueness([1])])
    (o, Dp) = from_lam(ast.run(to_lam(("a", "z")), D_with((("a", "x"),)), validate_obj=val))
    (p2, v) = o
    assert set(v) == {("a", "x"), ("a", "z")}        # both a-keyed tuples are in the violation set
    assert file_of(Dp) == {("a", "x")}                # D unchanged — no commit on an alethic violation

def test_uniqueness_allows_valid_commit():
    val = system.validate_of([C.uniqueness([1])])
    (o, Dp) = from_lam(ast.run(to_lam(("b", "y")), D_with((("a", "x"),)), validate_obj=val))
    (p2, v) = o
    assert v == ()
    assert file_of(Dp) == {("a", "x"), ("b", "y")}    # committed

def test_derive_reaches_fixpoint():
    # symmetric-closure rule alpha([2,1]); lfp adds ⟨b,a⟩ then converges (Knaster-Tarski)
    swap = _S(A("ALPHA"), _S(A("CONS"), A(2), A(1)))
    d = system.derive_of([swap])
    assert set(from_lam(apply(d, to_lam((("a", "b"),))))) == {("a", "b"), ("b", "a")}

def test_cell_isolation():
    # a command on FILE leaves a sibling cell OTHER untouched
    D = L.SEQ(L.CONS(ast.cell("FILE", to_lam((("a", "x"),))))(
              L.CONS(ast.cell("OTHER", to_lam((("keep", "me"),))))(L.NIL)))
    (_o, Dp) = from_lam(ast.run(to_lam(("b", "y")), D))
    other = [c for c in Dp if isinstance(c, tuple) and c[:2] == ("CELL", "OTHER")][0]
    assert set(other[2]) == {("keep", "me")}          # sibling cell preserved
