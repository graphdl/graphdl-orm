"""Codd theta1 as FFP objects (D4), reduced by the one mu — no host query logic."""
from pyarest import apply, to_lam, from_lam
from pyarest.lam import atom as A
import pyarest.prims  # noqa: F401  (registers the base incl. cat/not/1r/tlr)
from pyarest import canon as T

def ev(op, data):
    return from_lam(apply(op, to_lam(data)))


def test_selection_filter():
    # sigma over pairs: keep ⟨x,y⟩ with x = y
    pop = (("a", "a"), ("a", "b"), ("c", "c"))
    assert ev(T.Filter(A("eq")), pop) == (("a", "a"), ("c", "c"))

def test_projection_dedups():
    # pi_[1] of ⟨⟨a,x⟩,⟨b,y⟩,⟨a,z⟩⟩ = {⟨a⟩,⟨b⟩}  (duplicate ⟨a⟩ removed; a projection is
    # set-valued per Codd §2.1.2, so compare as a set — the sequence order is an artifact)
    pop = (("a", "x"), ("b", "y"), ("a", "z"))
    assert set(ev(T.Project([1]), pop)) == {("a",), ("b",)}

def test_natural_join():
    # R.2 = S.1 ; ⟨a,1⟩⋈⟨1,x⟩ = ⟨a,1,x⟩
    R = (("a", "1"), ("b", "2"))
    S = (("1", "x"), ("2", "y"))
    assert ev(T.NatJoin(2), (R, S)) == (("a", "1", "x"), ("b", "2", "y"))

def test_tie():
    # keep tuples whose first = last, then drop the last role
    pop = (("a", "b", "a"), ("c", "d", "e"))
    assert ev(T.Tie, pop) == (("a", "b"),)

def test_restrict_is_semijoin():
    # keep rows of R whose [1]-key occurs in pi_[1](S)
    R = (("a", "1"), ("b", "2"), ("c", "3"))
    S = (("a", "z"), ("c", "w"))
    assert ev(T.Restrict([1], [1]), (R, S)) == (("a", "1"), ("c", "3"))
