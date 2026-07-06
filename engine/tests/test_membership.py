"""S1, the paper's central identity, made a construction: "membership is the
characteristic function of P, so g ∈ P and P g are one act" (Def. pop). The mechanism is
eq. (1) itself: a fact carries its type as first element, so applying the population
metacomposes down to the type's definition, and the type's definition computes
membership: P:g = (ρ f₁):⟨P, g⟩ = (ρ FT):⟨f₁, ⟨P, g⟩⟩ = member:⟨g, P⟩. No new kernel
mechanism; the fact "is resolved by looking up its type" exactly as the paper says."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, system, defs
from pyarest.reduce import apply


def test_population_applied_to_a_fact_computes_membership():
    ft = "Order_was_placed_by_Customer"
    P = system.typed_population(ft, (("o1", "c1"), ("o2", "c2")))
    D = apply(ast.DefineIn(ft, system.membership_def()), L.SEQ(L.CONS(ast.cell("FILE", to_lam(())))(L.NIL)))
    with defs.step(D):
        g_in = system.typed_fact(ft, ("o1", "c1"))
        g_out = system.typed_fact(ft, ("o9", "c9"))
        assert from_lam(apply(P, g_in)) == "T"                # g ∈ P and P g are one act
        assert from_lam(apply(P, g_out)) == "F"


def test_membership_rides_metacomposition_not_a_new_mechanism():
    # the same population applied on BOTH evaluators agrees (it is ordinary reduction)
    from pyarest import reduce as R
    ft = "Person_likes_Person"
    P = system.typed_population(ft, (("a", "b"),))
    D = apply(ast.DefineIn(ft, system.membership_def()), L.SEQ(L.CONS(ast.cell("FILE", to_lam(())))(L.NIL)))
    with defs.step(D):
        g = system.typed_fact(ft, ("a", "b"))
        assert from_lam(R.apply_lambda(P, g)) == from_lam(R.apply(P, g)) == "T"
