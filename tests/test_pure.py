"""μ runs the object algebra with zero `if`: Scott dispatch, pure `fetch`
(Church-numeral equality), metacomposition, and the forms as FFP objects in
DEFS. CONST ≡ 2∘1 and CONS ≡ α·apply∘tl∘distr reduce through μ, never a Python
function.
"""
import pyarest                              # runs genesis (seeds DEFS)
from pyarest.objects import sym, seq, reify
from pyarest.reduce import apply

s = sym


def test_selector_1_primitive():
    assert reify(apply(s("1"), seq(s("a"), s("b"), s("c")))) == "a"


def test_selectors_2_3_are_ffp_objects():
    assert reify(apply(s("2"), seq(s("a"), s("b"), s("c")))) == "b"    # 1∘tl
    assert reify(apply(s("3"), seq(s("a"), s("b"), s("c")))) == "c"    # 1∘tl∘tl


def test_tail_primitive():
    assert reify(apply(s("tl"), seq(s("a"), s("b"), s("c")))) == ("b", "c")


def test_apply_primitive_returns_expression_mu_reduces():
    # apply:⟨1,⟨p,q⟩⟩ = (1:⟨p,q⟩) = p
    assert reify(apply(s("apply"), seq(s("1"), seq(s("p"), s("q"))))) == "p"


def test_const_by_metacomposition():
    # ⟨CONST,A⟩ : B → A
    assert reify(apply(seq(s("CONST"), s("A")), s("B"))) == "A"


def test_cons_by_metacomposition():
    # ⟨CONS,1,tl⟩ : ⟨p,q,r⟩ → ⟨1:⟨p,q,r⟩, tl:⟨p,q,r⟩⟩ = ⟨p, ⟨q,r⟩⟩
    assert reify(apply(seq(s("CONS"), s("1"), s("tl")), seq(s("p"), s("q"), s("r")))) == ("p", ("q", "r"))
