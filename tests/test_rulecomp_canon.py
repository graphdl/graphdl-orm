"""The rule compilers from the shared source (wave S2). A compiled rule is the fold
of joins over the body atoms (Def. derive: a role path projected onto a head),
comparator filters after the joins, the head positions projected — built at
reduction by a WHILE over the atom list carrying ⟨tree, width, rest⟩, the same
build-a-tree-as-data discipline as every canonical builder, with Pop's WHILE as the
precedent. The atom spec is ⟨ft, width, join?⟩ with join? empty for the linear chain
(NatJoin on the running tuple's last column) or ⟨pairs, keep⟩ for the general Codd
join. cmp_filter is the comparator predicate pair (literal and column forms). The
gates hold the canonical names to the host compilers on identical bodies, and one
absolute closure run pins semantics end to end."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import ast, defs, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _D(*cells):
    return to_lam(tuple(("CELL", n, v) for (n, v) in cells))


def _eq_rule(param, host_obj, D):
    """A compiled rule applies to D itself (run_rules: rid:D)."""
    with defs.step(L.SEQ(L.NIL)):
        built = apply(A("system:compile_rule"), param)
    with defs.step(D):
        got = from_lam(apply(built, D))
        want = from_lam(apply(host_obj, D))
    assert got == want, f"compile_rule: {got!r} != {want!r}"
    return got


def test_cmp_filter_literal_and_column_forms():
    # the contract: cmp_filter is the bare PREDICATE (compile_rule Filter-wraps it)
    with defs.step(L.SEQ(L.NIL)):
        p = apply(A("system:cmp_filter_lit"), S(A("gt"), A(1), to_lam(4)))
        assert from_lam(apply(p, to_lam((7, 7)))) == "T"
        assert from_lam(apply(p, to_lam((3, 2)))) == "F"
        assert from_lam(apply(system.cmp_filter("gt", 1, lit=4),
                              to_lam((7, 7)))) == "T"
        p2 = apply(A("system:cmp_filter_col"), S(A("lt"), A(1), A(2)))
        assert from_lam(apply(p2, to_lam((1, 5)))) == "T"
        assert from_lam(apply(p2, to_lam((7, 7)))) == "F"
        assert from_lam(apply(system.cmp_filter("lt", 1, col2=2),
                              to_lam((1, 5)))) == "T"


def test_compile_rule_single_atom_projection():
    D = _D(("FT", (("a", "b"), ("c", "d"))))
    param = S(S(S(A("FT"), to_lam(2), to_lam(()))),           # one atom, width 2, linear
              to_lam((2, 1)),                                 # head: swap
              to_lam(()))                                     # no filters
    got = _eq_rule(param, system.compile_rule(["FT"], [2, 1], [2]), D)
    assert set(got) == {("b", "a"), ("d", "c")}


def test_compile_rule_linear_chain_and_filters():
    D = _D(("R", (("a", "b"), ("x", "y"))), ("Sx", (("b", 3), ("y", 9))))
    f = system.cmp_filter("gt", 3, lit=5)
    param = S(S(S(A("R"), to_lam(2), to_lam(())),
                S(A("Sx"), to_lam(2), to_lam(()))),
              to_lam((1, 3)),
              S(f))
    got = _eq_rule(param, system.compile_rule(["R", "Sx"], [1, 3], [2, 2], [f]), D)
    assert set(got) == {("x", 9)}


def test_compile_rule_general_join_spec():
    D = _D(("P", ((1, "x", 5),)), ("Q", ((5, "u", "k"),)))
    param = S(S(S(A("P"), to_lam(3), to_lam(())),
                S(A("Q"), to_lam(3), S(to_lam(((3, 1),)), to_lam((2, 3))))),
              to_lam((1, 5)),
              to_lam(()))
    got = _eq_rule(param, system.compile_rule(["P", "Q"], [1, 5], [3, 3],
                                              joins=[(((3, 1),), (2, 3))]), D)
    assert set(got) == {(1, "k")}
