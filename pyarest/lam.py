"""Layer 0 — the lambda base.

ZFC is encoded into the lambda calculus; in the lambda calculus there is only
the lambda. Every name below is a pure Python ``lambda``: abstraction and
application, nothing else — no ``def`` body, no ``if``, no loop, no builtin
data structure. This is the irreducible base of the whole tower.

Backus's FP objects, primitives, and combining forms (Layer 1) are expressed
as lambda terms over these combinators. Everything above Backus is Backus AST.
Because the layers above name only these combinators, the base is an
*interface*: replace the encoding here and the algebra above is unchanged.

Encodings:
  * booleans  — Church:  TRUE = λt.λf.t,  FALSE = λt.λf.f
  * pairs     — Church:  ⟨a,b⟩ = λs. s a b
  * lists     — right fold (Böhm–Berarducci): l = λc.λn. c x1 (c x2 (... n))
  * recursion — the call-by-value Z fixpoint combinator
"""

# --- Combinators ----------------------------------------------------------
I = lambda x: x                                   # identity            I x = x
K = lambda x: lambda _y: x                        # const / Church TRUE K x y = x
S = lambda f: lambda g: lambda x: f(x)(g(x))      # substitution
B = lambda f: lambda g: lambda x: f(g(x))         # composition  (∘)
C = lambda f: lambda x: lambda y: f(y)(x)         # flip

# --- Church booleans ------------------------------------------------------
TRUE = K                                          # λt.λf. t
FALSE = lambda _t: lambda f: f                    # λt.λf. f   (= K I)
IF = lambda p: lambda a: lambda b: p(a)(b)        # p a b
NOT = lambda p: p(FALSE)(TRUE)
AND = lambda p: lambda q: p(q)(FALSE)
OR = lambda p: lambda q: p(TRUE)(q)

# --- Church pairs ---------------------------------------------------------
PAIR = lambda a: lambda b: lambda s: s(a)(b)      # ⟨a,b⟩
FST = lambda p: p(TRUE)
SND = lambda p: p(FALSE)

# --- Church lists (right-fold encoding) -----------------------------------
# NIL c n = n ;  (CONS h t) c n = c h (t c n)
NIL = lambda _c: lambda n: n
CONS = lambda h: lambda t: lambda c: lambda n: c(h)(t(c)(n))
ISNIL = lambda l: l(lambda _h: lambda _acc: FALSE)(TRUE)
HEAD = lambda l: l(lambda h: lambda _acc: h)(NIL)  # head of a non-empty list
# TAIL by the pairing shift: fold ⟨built-so-far, built-without-first⟩.
TAIL = lambda l: SND(
    l(lambda h: lambda p: PAIR(CONS(h)(FST(p)))(FST(p)))(PAIR(NIL)(NIL))
)
FOLDR = lambda op: lambda z: lambda l: l(op)(z)    # / (insert right)
MAP = lambda f: lambda l: l(lambda h: lambda acc: CONS(f(h))(acc))(NIL)  # α

# --- Church numerals ------------------------------------------------------
# n = λf.λx. f^n(x).  Atom identities are numerals, so eq is numeral equality
# and DEFS lookup (Backus's fetch) is pure λ — no host comparison, no `if`.
ZERO = lambda _f: lambda x: x
SUCC = lambda n: lambda f: lambda x: f(n(f)(x))
ISZERO = lambda n: n(lambda _k: FALSE)(TRUE)
PRED = lambda n: lambda f: lambda x: n(lambda g: lambda h: h(g(f)))(lambda _u: x)(lambda u: u)
SUB = lambda m: lambda n: n(PRED)(m)
LEQ = lambda m: lambda n: ISZERO(SUB(m)(n))
EQ = lambda m: lambda n: AND(LEQ(m)(n))(LEQ(n)(m))

# --- Recursion: the call-by-value Z fixpoint combinator -------------------
# Z f = f (λv. Z f v).  Engine of `while` and of derive = lfp(F).
Z = lambda f: (lambda x: f(lambda v: x(x)(v)))(lambda x: f(lambda v: x(x)(v)))

# while p f x : iterate x → f x while p x holds. CBV-guarded with unit thunks.
WHILE = lambda p: lambda f: Z(
    lambda rec: lambda x: p(x)(lambda _u: rec(f(x)))(lambda _u: x)(I)
)
