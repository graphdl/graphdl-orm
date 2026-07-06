"""The ast cell accessors from the shared source, gated strictly: the canonical NAME
applied to the cell name (DefineIn to the pair) must produce the exact accessor
semantics of Backus 13.3.4/13.3.5 on hand-built stores, and the python/ast.py
wrapper must agree. The stack discipline is the load-bearing detail: cells of one
name form a LIFO stack, fetch reads the top, pop removes ONLY the top, purge removes
all, store is push-after-pop so deeper same-named cells survive."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import ast, defs
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _D(*cells):
    return to_lam(tuple(("CELL", n, v) for (n, v) in cells))


def _both(name, param, x):
    """Apply the canonical name AND the wrapper; both must agree."""
    D = L.SEQ(L.NIL)
    with defs.step(D):
        via_name = from_lam(apply(apply(A(name), param), x))
    return via_name


def test_fetch_reads_the_top_of_the_stack_or_the_mark():
    D = _D(("a", ("x",)), ("b", ("y",)), ("a", ("deep",)))
    assert _both("ast:Fetch", A("a"), D) == ("x",)            # the TOP a-cell
    assert _both("ast:Fetch", A("zz"), D) == "#"              # absent: unaddressable
    assert from_lam(apply(ast.Fetch("b"), D)) == ("y",)       # the wrapper agrees


def test_fetchpop_defaults_the_absent_cell_to_the_empty_population():
    D = _D(("a", (("r",),)))
    assert _both("ast:FetchPop", A("zz"), D) == ()
    assert _both("ast:FetchPop", A("a"), D) == (("r",),)
    assert from_lam(apply(ast.FetchPop("zz"), D)) == ()


def test_pop_removes_only_the_top_purge_removes_all():
    D = _D(("a", ("top",)), ("b", ("keep",)), ("a", ("deep",)))
    popped = _both("ast:Pop", A("a"), D)
    assert popped == (("CELL", "b", ("keep",)), ("CELL", "a", ("deep",)))
    purged = _both("ast:Purge", A("a"), D)
    assert purged == (("CELL", "b", ("keep",)),)
    assert from_lam(apply(ast.Pop("a"), D)) == popped
    assert from_lam(apply(ast.Purge("a"), D)) == purged


def test_store_is_push_after_pop_deeper_cells_survive():
    D = _D(("a", ("old",)), ("a", ("deep",)))
    stored = _both("ast:Store", A("a"), S(to_lam(("new",)), D))
    assert stored == (("CELL", "a", ("new",)),
                      ("CELL", "a", ("deep",)))               # top replaced, deep kept
    assert from_lam(apply(ast.Store("a"), S(to_lam(("new",)), D))) == stored


def test_definein_stores_the_object_as_an_ordinary_cell():
    D = _D(("x", ("v",)))
    obj = S(A("COMP"), A("not"), A("null"))
    with defs.step(L.SEQ(L.NIL)):
        out = from_lam(apply(apply(A("ast:DefineIn"),
                                   S(A("mydef"), obj)), D))
    assert out == (("CELL", "mydef", ("COMP", "not", "null")),
                   ("CELL", "x", ("v",)))
    assert from_lam(apply(ast.DefineIn("mydef", obj), D)) == out
