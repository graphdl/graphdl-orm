"""The bottom discipline (Backus §11.2.1, §13.3.1): every function is ⊥-preserving —
f:⊥ = ⊥ for EVERY f — and a sequence containing ⊥ IS ⊥ (the object-domain axiom).
Both engines must agree: the λ kernel (ground truth) and the δ fast-path."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import reduce as R


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def both(f, x):
    xl = to_lam(x)
    return from_lam(R.apply_lambda(f, xl)), from_lam(R.apply(f, xl))


BOT = "⊥"
_4 = A(4)  # 4:⟨a,b⟩ = ⊥ — the canonical ⊥ producer below


def test_alpha_over_bottom_is_bottom():
    # α(id) ∘ 4 : ⟨a,b⟩ — the operand of α reduces to ⊥; ⊥ must never be iterated as data
    f = S(A("COMP"), S(A("ALPHA"), A("id")), _4)
    assert both(f, ("a", "b")) == (BOT, BOT)


def test_construction_containing_bottom_is_bottom():
    # [id, 4] : ⟨a,b⟩ = ⟨⟨a,b⟩, ⊥⟩ = ⊥  (§11.2.1: ⟨…,⊥,…⟩ IS ⊥)
    f = S(A("CONS"), A("id"), _4)
    assert both(f, ("a", "b")) == (BOT, BOT)


def test_every_function_is_bottom_preserving():
    cases = [
        S(A("COMP"), A("id"), _4),                                 # id:⊥
        S(A("COMP"), A("tl"), _4),                                 # tl:⊥
        S(A("COMP"), A("length"), _4),                             # length:⊥
        S(A("COMP"), S(A("CONST"), A("k")), _4),                   # k̄:⊥ = ⊥ (§11.2.2)
        S(A("COMP"), S(A("ALPHA"), A("id")), _4),                  # αf:⊥
        S(A("COMP"), S(A("INSERT"), A("cat")), S(A("CONS"), A("id"), _4)),  # /f over a ⊥ seq
    ]
    for f in cases:
        assert both(f, ("a", "b")) == (BOT, BOT)


def test_and_or_are_defined_on_TF_only():
    # Backus's and/or are boolean functions: anything outside {T,F} is ⊥, never F
    assert both(A("and"), (5, "T")) == (BOT, BOT)
    assert both(A("or"), ("x", "F")) == (BOT, BOT)
