"""Aggregates in rule heads (Def. derive sanctions them verbatim: 'an aggregate
reducing a finite bag to one scalar'). The clause surface `<out> is the <op> of
<source>` is the readings corpus's numeric-aggregation shape (derivation.md shape 7:
`<role> is the <op> of <target> where <body>`, ops count/sum/avg/min/max; here the bag
is scoped by the rule's own conjuncts instead of a `where` suffix), and the form is
Halpin's own gloss of aggregation ("n is the count of all the facts satisfying
condition", Logical Data Modeling Part 13). Semantics per Halpin/Curland's ORM-to-
datalog mapping: the aggregate stratum sits above the positive closure (agg<<>> over a
derived predicate), the head is functional per group, so recompute REPLACES — the old
engine's documented misfold (a stale larger min surviving union-merge over a growing
source) is this suite's regression case. min/max only for now: they are what the
executed scenarios judge; count/sum/avg fall to ruleDiag until landed."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


MODEL = """Node(.Id) is an entity type.
Cost is a value type.
Node moves to Node at Cost.
Node cheapest to Node at Cost.
Node1 cheapest to Node2 at Cost3 if Node1 moves to Node2 at Cost2 and Cost3 is the min of Cost2.
"""


def test_min_folds_to_one_scalar_per_group():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    assert rep["rule_diagnostics"] == []
    D = apply(ast.Store("Node_moves_to_Node_at_Cost"),
              S(to_lam((("a", "b", 5), ("a", "b", 3), ("a", "c", 7))), D))
    D = system.run_rules(D)
    assert _cell(from_lam(D), "Node_cheapest_to_Node_at_Cost") == \
        {("a", "b", 3), ("a", "c", 7)}


def test_a_better_minimum_supersedes_the_stale_one():
    # the old engine's misfold, as our regression: the aggregate head REPLACES on
    # recompute, so a later cheaper edge supersedes the stored minimum
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.Store("Node_moves_to_Node_at_Cost"),
              S(to_lam((("a", "b", 5), ("a", "b", 3))), D))
    D = system.run_rules(D)
    assert _cell(from_lam(D), "Node_cheapest_to_Node_at_Cost") == {("a", "b", 3)}
    D = apply(A(2), system.create(D, "Node_moves_to_Node_at_Cost", to_lam(("a", "b", 2))))
    D = system.run_rules(D)
    assert _cell(from_lam(D), "Node_cheapest_to_Node_at_Cost") == {("a", "b", 2)}


def test_max_folds_too():
    MODEL2 = MODEL.replace("cheapest", "dearest").replace("the min of", "the max of")
    D, rep = forml.compile_model(MODEL2)
    assert rep["rule_diagnostics"] == []
    D = apply(ast.Store("Node_moves_to_Node_at_Cost"),
              S(to_lam((("a", "b", 5), ("a", "b", 3))), D))
    D = system.run_rules(D)
    assert _cell(from_lam(D), "Node_dearest_to_Node_at_Cost") == {("a", "b", 5)}
