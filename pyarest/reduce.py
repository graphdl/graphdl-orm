"""μ, the meaning function — the least fixed point of τ (Backus §13.4), by
dispatch. No `if`: every branch is an object applying its own handler;
recursion is the Z fixpoint; DEFS lookup is the pure `fetch`.

The operator is expanded first, then it dispatches itself — an atom fetches its
definition, a sequence metacomposes, a prim applies its host λ. A primitive's
result is itself reduced again (`μ((ρy)(μz))`), because `apply`/`COMP`/… return
application *expressions*, not finished objects.
"""
from . import lam as L
from .objects import atom, sq, app, bot, prim, seq2, is_default
from .defs import fetch, defs

# operator is an atom name a: fetch its contents, then dispatch the contents
_redatom = lambda self: lambda a: lambda x: (lambda c: c
    (lambda b: is_default(atom(b))(lambda _u: bot)(lambda _u: self(app(atom(b))(x)))(L.I))  # atom (alias / #→⊥)
    (lambda s: self(app(sq(s))(x)))                                                          # FFP object → reduce
    (lambda o: lambda x2: bot)                                                               # app → ⊥
    (bot)                                                                                     # ⊥
    (lambda g: self(g(self(x))))                                                              # prim → μ((ρy)(μx))
)(fetch(a)(defs()))

# dispatch the already-reduced operator f applied to operand x
_redop = lambda self: lambda f: lambda x: (f
    (lambda a: _redatom(self)(a)(x))                                    # atom name
    (lambda s: self(app(L.HEAD(s))(seq2(sq(s))(x))))                    # metacomposition
    (lambda o: lambda x2: bot)                                          # app → ⊥
    (bot)                                                              # ⊥
    (lambda g: self(g(self(x))))                                        # prim → μ((ρy)(μx))
)

meaning = L.Z(lambda self: lambda e: (e
    (lambda a: atom(a))                                                # μ(atom) = atom
    (lambda s: sq(L.MAP(self)(s)))                                     # μ(seq)  = α μ
    (lambda o: lambda x: _redop(self)(self(o))(x))                     # μ(app)  = reduce
    (bot)                                                             # μ(⊥) = ⊥
    (lambda g: prim(g))                                                # μ(prim) = prim
))


def apply(f, x):
    """The FFP application (f:x), evaluated: μ(f:x)."""
    return meaning(app(f)(x))
