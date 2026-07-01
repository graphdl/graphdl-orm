"""Layer 0 is real lambda calculus, and Backus's FP is expressible in it.

The first block exercises the base combinators. The second block *builds the
Backus primitives and combining forms as lambda terms over the base* — the
thing that was missing: selectors are HEAD∘TAIL^(n-1), α is a fold, `while`
is the Z fixpoint. No procedural code participates in the values under test;
the decoders below only read a finished Church value back into Python so a
plain ``assert`` can see it.
"""
from pyarest import lam as L


# --- decoders (test-only: Church value -> Python, for assertions) ---------
def to_bool(b):
    return b(True)(False)


def to_list(l):
    # right fold with Python cons; list elements are raw Python values here
    return l(lambda h: lambda acc: [h] + acc)([])


# --- Layer 0: the base combinators ----------------------------------------
def test_combinators():
    assert L.I(5) == 5
    assert L.K(1)(2) == 1
    assert L.S(lambda x: lambda y: x + y)(lambda x: x)(3) == 6   # S add id 3
    assert L.B(lambda x: x + 1)(lambda x: x * 2)(3) == 7          # (+1)∘(*2)
    assert L.C(lambda a: lambda b: a - b)(1)(9) == 8              # flip: 9-1


def test_church_booleans():
    assert to_bool(L.TRUE) is True
    assert to_bool(L.FALSE) is False
    assert to_bool(L.NOT(L.TRUE)) is False
    assert to_bool(L.AND(L.TRUE)(L.TRUE)) is True
    assert to_bool(L.AND(L.TRUE)(L.FALSE)) is False
    assert to_bool(L.OR(L.FALSE)(L.TRUE)) is True
    assert L.IF(L.TRUE)("a")("b") == "a"
    assert L.IF(L.FALSE)("a")("b") == "b"


def test_church_pairs():
    p = L.PAIR("x")("y")
    assert L.FST(p) == "x"
    assert L.SND(p) == "y"


def test_church_lists():
    xs = L.CONS(1)(L.CONS(2)(L.CONS(3)(L.NIL)))
    assert to_list(xs) == [1, 2, 3]
    assert to_bool(L.ISNIL(L.NIL)) is True
    assert to_bool(L.ISNIL(xs)) is False
    assert L.HEAD(xs) == 1
    assert to_list(L.TAIL(xs)) == [2, 3]
    assert to_list(L.TAIL(L.CONS(1)(L.NIL))) == []   # tl of a singleton = φ


def test_fixpoint_combinator():
    # Z is the engine of `while`/lfp; prove it computes a real fixpoint.
    fact = L.Z(lambda rec: lambda n: 1 if n == 0 else n * rec(n - 1))
    assert fact(5) == 120


# --- Layer 1: Backus FP *as lambda terms over the base* --------------------
def sel(n):
    # selector n  ≡  1 ∘ tl^(n-1)  ≡  HEAD ∘ TAIL∘…∘TAIL
    f = L.HEAD
    for _ in range(n - 1):          # construction-time composition, not runtime
        f = L.B(f)(L.TAIL)
    return f


def test_selectors_are_composed_from_head_and_tail():
    xs = L.CONS("a")(L.CONS("b")(L.CONS("c")(L.NIL)))
    assert sel(1)(xs) == "a"
    assert sel(2)(xs) == "b"
    assert sel(3)(xs) == "c"


def test_tl_and_null_are_base_terms():
    xs = L.CONS("a")(L.CONS("b")(L.NIL))
    assert to_list(L.TAIL(xs)) == ["b"]          # tl
    assert to_bool(L.ISNIL(L.NIL)) is True        # null:φ
    assert to_bool(L.ISNIL(xs)) is False


def test_cond_is_church_if():
    # (p → f ; g) : x   as a lambda term
    cond = lambda p: lambda f: lambda g: lambda x: L.IF(p(x))(f)(g)(x)
    pos = lambda n: L.TRUE if n > 0 else L.FALSE
    got = cond(pos)(lambda n: "pos")(lambda n: "nonpos")
    assert got(5) == "pos"
    assert got(-1) == "nonpos"


def test_alpha_is_map_and_insert_is_foldr():
    xs = L.CONS(1)(L.CONS(2)(L.CONS(3)(L.NIL)))
    doubled = L.MAP(lambda n: n * 2)(xs)                  # αf
    assert to_list(doubled) == [2, 4, 6]
    total = L.FOLDR(lambda a: lambda b: a + b)(0)(xs)     # /+
    assert total == 6


def test_while_is_the_z_fixpoint():
    # while (x>0) (x-1) : 5  →  0
    pos = lambda n: L.TRUE if n > 0 else L.FALSE
    assert L.WHILE(pos)(lambda n: n - 1)(5) == 0
