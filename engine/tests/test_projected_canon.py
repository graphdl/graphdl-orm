"""The projected subset/exclusion deontic checkers, moved from host-only
compositions into constraints.canon (2026-07-09 canon-completeness audit).
Gated by the ABSOLUTE result (intersection.md: authorship tests demand the exact
value, never a reference-bearing tautology): each canonical NAME applied to
<cell, proj_p, proj_c[, pos, lit]> builds a checker that, on a fixed synthetic
<P, D>, yields the exact violation set. Because the builders compose only
differential-covered primitives, the Rust host reduces the identical bytes.

Synthetic world: antecedent A (target population P) = <a,x>,<b,y>,<c,irreversible>;
sibling head cell 'Head' = <a,z>. proj_p = proj_c = [1] (the entity role);
value filter selects position 2 == 'irreversible' (only row c)."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


P = to_lam((("a", "x"), ("b", "y"), ("c", "irreversible")))
D = to_lam((("CELL", "Head", (("a", "z"),)),))
IN = S(P, D)


def check(name, *param):
    built = R(A(name), to_lam(tuple(param)))
    return sorted(from_lam(R(built, IN)))


def test_subset_projected():
    # π₁(P) ∖ π₁(Head) = {a,b,c} ∖ {a} = {b,c}
    assert check("constraints:scoped_subset_projected", "Head", (1,), (1,)) == [("b",), ("c",)]


def test_exclusion_projected():
    # π₁(P) ∩ π₁(Head) = {a,b,c} ∩ {a} = {a}
    assert check("constraints:scoped_exclusion_projected", "Head", (1,), (1,)) == [("a",)]


def test_subset_projected_filtered():
    # π₁(σ_{2=irreversible}(P)) ∖ π₁(Head) = {c} ∖ {a} = {c}
    assert check("constraints:scoped_subset_projected_filtered",
                 "Head", (1,), (1,), 2, "irreversible") == [("c",)]


def test_exclusion_projected_filtered():
    # π₁(σ_{2=irreversible}(P)) ∩ π₁(Head) = {c} ∩ {a} = {}
    assert check("constraints:scoped_exclusion_projected_filtered",
                 "Head", (1,), (1,), 2, "irreversible") == []
