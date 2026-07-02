"""The lambda kernel: Backus's base reduced by mu = Y(tau), the genuine LFP.

Everything here is FFP objects (sequences of atoms) reduced by the one mu. No native
objects, no Python recursion in the semantics — the recursion is the Y combinator.
"""
import pyarest.lam as L
from pyarest import apply, meaning, to_lam, from_lam, atom
import pyarest.prims  # noqa: F401  (registers the Backus base into DEFS on import)

A = atom
def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)
def ev(f, x):
    return from_lam(apply(f, to_lam(x)))


# ---- primitives ----
def test_selectors_and_tl():
    assert ev(A(1), ("a", "b", "c")) == "a"
    assert ev(A(3), ("a", "b", "c")) == "c"
    assert ev(A(4), ("a", "b", "c")) == "⊥"          # out of range -> bottom
    assert ev(A("tl"), ("a", "b", "c")) == ("b", "c")

def test_predicates():
    assert ev(A("atom"), "x") == "T"
    assert ev(A("atom"), ("a", "b")) == "F"
    assert ev(A("null"), ()) == "T"
    assert ev(A("null"), ("a",)) == "F"
    assert ev(A("eq"), ("a", "a")) == "T"
    assert ev(A("eq"), ("a", "b")) == "F"

# ---- combining forms (controlling operators driven by metacomposition) ----
def test_comp_is_metacomposition():
    # ⟨COMP, 1, tl⟩ : ⟨a,b,c⟩ = 1:(tl:⟨a,b,c⟩) = 1:⟨b,c⟩ = b
    assert ev(S(A("COMP"), A(1), A("tl")), ("a", "b", "c")) == "b"

def test_cons_is_construction():
    # ⟨CONS, 2, 1⟩ : ⟨a,b⟩ = ⟨2:⟨a,b⟩, 1:⟨a,b⟩⟩ = ⟨b,a⟩  (swap)
    assert ev(S(A("CONS"), A(2), A(1)), ("a", "b")) == ("b", "a")

def test_const_is_constant():
    assert ev(S(A("CONST"), A("k")), ("anything",)) == "k"

def test_cond_is_conditional():
    pick = S(A("COND"), A("atom"), S(A("CONST"), A("isatom")), S(A("CONST"), A("isseq")))
    assert ev(pick, "q") == "isatom"
    assert ev(pick, ("a", "b")) == "isseq"

def test_alpha_is_apply_to_all():
    # ⟨ALPHA, 1⟩ : ⟨⟨a,b⟩,⟨c,d⟩⟩ = ⟨1:⟨a,b⟩, 1:⟨c,d⟩⟩ = ⟨a,c⟩
    assert ev(S(A("ALPHA"), A(1)), (("a", "b"), ("c", "d"))) == ("a", "c")

def test_insert_is_reduce():
    # ⟨INSERT, apndl⟩ folds ⟨1,⟨2,⟨3, ()⟩⟩⟩-style; use it to concat: /apndr not needed here.
    # /1 : ⟨x⟩ = x ; and /f on a singleton returns the element
    assert ev(S(A("INSERT"), A(1)), ("only",)) == "only"

# ---- Backus's algebra-of-programs law (§12.2), as FFP objects reduced by mu ----
def test_backus_12_2_law():
    f, g, h = A(1), A(2), A("tl")
    lhs = S(A("COMP"), S(A("CONS"), f, g), h)                 # ([1,2] ∘ tl)
    rhs = S(A("CONS"), S(A("COMP"), f, h), S(A("COMP"), g, h)) # [1∘tl, 2∘tl]
    x = ("p", "q", "r", "s")
    assert ev(lhs, x) == ev(rhs, x) == ("q", "r")

# ---- mu is a genuine least fixed point: reducing a normal form is stable ----
def test_mu_is_idempotent_on_normal_forms():
    comp = S(A("COMP"), A(1), A("tl"))
    r = apply(comp, to_lam(("a", "b", "c")))                  # already a normal form (an atom)
    assert from_lam(meaning(r)) == from_lam(r) == "b"         # mu(mu e) = mu e
