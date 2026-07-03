"""Wave S2b: the semi-naive delta variant and the aggregate fold from the shared
source. compile_rule_delta is the same WHILE-over-atoms fold with one substitution:
atom delta_at reads the round's DELTA (selector 1 of the ⟨Δ, D⟩ operand) while every
other atom fetches its cell composed with selector 2 — Bancilhon-Ramakrishnan's
inner join, so the canonical builder threads an atom index through the fold state.
compile_agg_rule shares the join prefix and adds the per-group fold (Def. derive:
an aggregate reducing a finite bag to one scalar): min folds le, max folds ge, the
group selectors come from selrow, and the head REPLACES on recompute (enforced by
run_rules' stratified pass, unchanged here)."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import defs, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _D(*cells):
    return to_lam(tuple(("CELL", n, v) for (n, v) in cells))


def _atoms(*specs):
    out = []
    for (ft, w, j) in specs:
        spec = to_lam(()) if j is None else S(to_lam(tuple(tuple(p) for p in j[0])),
                                              to_lam(tuple(j[1])))
        out.append(S(A(ft), to_lam(w), spec))
    return S(*out)


def test_delta_substitutes_exactly_one_atom():
    D = _D(("R", (("a", "b"), ("x", "y"))), ("Sx", (("b", 3), ("y", 9))))
    delta = to_lam((("a", "b"),))                             # only the a-row is new
    for at in (0, 1):
        host = system.compile_rule_delta(["R", "Sx"], [1, 3], at, [2, 2])
        param = S(_atoms(("R", 2, None), ("Sx", 2, None)),
                  to_lam((1, 3)), to_lam(()), to_lam(at + 1))
        with defs.step(L.SEQ(L.NIL)):
            built = apply(A("system:compile_rule_delta"), param)
        with defs.step(D):
            got = from_lam(apply(built, S(delta if at == 0 else to_lam((("b", 3),)),
                                          D)))
            want = from_lam(apply(host, S(delta if at == 0 else to_lam((("b", 3),)),
                                          D)))
        assert got == want, f"delta_at={at}: {got!r} != {want!r}"
        if at == 0:
            assert set(got) == {("a", 3)}                     # Δ-row joined forward
        else:
            assert set(got) == {("a", 3)}                     # Δ-row joined backward


def test_agg_rule_min_and_max_fold_per_group():
    D = _D(("E", (("a", "b", 5), ("a", "b", 3), ("a", "c", 7))))
    for op, expect in (("min", {("a", "b", 3), ("a", "c", 7)}),
                       ("max", {("a", "b", 5), ("a", "c", 7)})):
        host = system.compile_agg_rule(["E"], [1, 2], 3, op, [3])
        param = S(_atoms(("E", 3, None)), to_lam((1, 2)), to_lam(3), A(op),
                  to_lam(()))
        with defs.step(L.SEQ(L.NIL)):
            built = apply(A("system:compile_agg_rule"), param)
        with defs.step(D):
            got = from_lam(apply(built, D))
            want = from_lam(apply(host, D))
        assert got == want, f"{op}: {got!r} != {want!r}"
        assert set(got) == expect, op
