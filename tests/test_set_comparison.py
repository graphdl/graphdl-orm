"""Set-comparison constraints made executable over a participation population ⟨entity, clause⟩:
exclusion (at most one), inclusive-or / disjunctive mandatory (at least one), exclusive-or
(exactly one). They reduce to theta1 uniqueness / setminus and enforce as (rho c):P = V_c."""
from pyarest import from_lam, to_lam, apply
from pyarest.lam import atom as A
import pyarest.prims  # noqa: F401
from pyarest import constraints as C, forml


def test_subset_is_modus_ponens():
    # 'if A then B': antecedent facts whose consequent does not hold
    a, b = (("x",), ("y",), ("z",)), (("x",), ("y",))
    assert set(from_lam(apply(C.subset(), to_lam((a, b))))) == {("z",)}


def test_equality_symmetric_difference():
    a, b = (("x",), ("y",)), (("y",), ("z",))
    assert set(from_lam(apply(C.equality(), to_lam((a, b))))) == {("x",), ("z",)}


def test_if_then_classifies_as_subset_not_derivation():
    assert forml.classify("If some Message matches some Rep then that Message is sent by that Rep.")[0] == "subset"
    assert forml.classify("Message is with Phone if and only if Rep has Phone.")[0] == "equality"


def ev(obj, data):
    return from_lam(apply(obj, to_lam(data)))


def test_exclusion_at_most_one_clause():
    part = (("a", "c1"), ("a", "c2"), ("b", "c1"))            # a participates in two clauses
    assert set(ev(C.exclusion(), part)) == {("a", "c1"), ("a", "c2")}


def test_inclusive_or_at_least_one_clause():
    # universe ∖ players: b participates in no clause
    assert set(ev(C.inclusive_or(), ((("a",), ("b",), ("c",)), (("a",), ("c",))))) == {("b",)}


def test_exclusive_or_exactly_one_clause():
    univ = (("a",), ("b",), ("c",), ("d",))
    part = (("a", "c1"), ("a", "c2"), ("b", "c1"))            # a: 2 clauses, b: 1, c/d: 0
    assert set(ev(C.exclusive_or(), (univ, part))) == {("c",), ("d",), ("a",)}   # not exactly one


def test_compile_set_comparison_defines_enforceable_object():
    model = ("For each Message, at most one of the following holds: "
             "that Message is with some external Phone; that Message is with some internal Email.")
    _D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    part = (("m1", "phone"), ("m1", "email"), ("m2", "phone"))   # m1 in both clauses
    v = from_lam(apply(A("Message_excl"), to_lam(part)))
    assert set(v) == {("m1", "phone"), ("m1", "email")}      # exclusion violation, straight from compile
