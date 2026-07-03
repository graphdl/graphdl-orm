"""Codd's adequate collection theta1 (Codd §2.2) — the Python BINDING of the
canonical definitions in shared/theta.py (intersection source, one file for every
host). The closed objects here ARE the canon values, loaded from the shared file;
the parameterized constructors APPLY the canonical builders through the reducer
(canon boots with the package, so the names resolve). Nothing is defined twice:
this module binds, it does not author. JoinOn and Restrict are the remainder of
the toolchain, composing canon pieces host-side until their COND-over-null
builders land in the shared file. "Navigation needs no separate query language" —
each operator is a rho-application over the population P.
"""
from . import canon as _canon
from . import lam as L
from .lam import atom as A, to_lam
from .reduce import apply as _apply

def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)

_COMP, _CONS = A("COMP"), A("CONS")
_ALPHA = A("ALPHA")
_EQ, _NOT, _DISTL, _DISTR = A("eq"), A("not"), A("distl"), A("distr")
_CAT = A("cat")
_1, _2 = A(1), A(2)                                          # selectors are numeric atoms

_C = dict(_canon.read("theta.py"))                           # the shared file, verbatim

member = _member = _C["theta:member"]
dedup = _dedup = _C["theta:dedup"]
flatten = _flatten = _C["theta:flatten"]
setminus = _C["theta:setminus"]
Tie = _C["theta:Tie"]


def Filter(p):
    """Codd selection sigma_p: the canonical builder applied to p (shared/theta.py)."""
    return _apply(A("theta:Filter"), p)


def NatJoin(i):
    """Codd natural join R*S (§2.1.3), joining R.i = S.1: the canonical builder
    applied to the selector."""
    return _apply(A("theta:NatJoin"), A(i))


def Project(cols):
    """Codd projection pi_L (§2.1.2): the canonical builder applied to the selector
    row ⟨c1..ck⟩."""
    return _apply(A("theta:Project"), to_lam(tuple(cols)))


def JoinOn(pairs, keep):
    """Codd's join (§2.1.3) in its general equi form: R ⋈ S on {R.ri = S.si} for the
    (ri, si) in `pairs`, emitting r ++ s[keep] (the fresh columns, in clause order).
    Empty `pairs` is the degenerate cross product; empty `keep` is the semijoin.
    Toolchain remainder: composes canon pieces host-side pending its COND builder.
        match   = eq∘[⟨ri…⟩∘1, ⟨si…⟩∘2]
        combine = cat∘[1, ⟨keep…⟩∘2]        (just 1 when keep is empty)
        R⋈S     = flatten ∘ α( α(combine) ∘ Filter(match) ∘ distl ) ∘ distr
    """
    if keep:
        ksel = _S(_CONS, *tuple(A(i) for i in keep))
        combine = _S(_COMP, _CAT, _S(_CONS, _1, _S(_COMP, ksel, _2)))
    else:
        combine = _1
    parts = [_S(_ALPHA, combine)]
    if pairs:
        rsel = _S(_CONS, *tuple(_S(_COMP, A(r), _1) for (r, _s) in pairs))
        ssel = _S(_CONS, *tuple(_S(_COMP, A(s), _2) for (_r, s) in pairs))
        parts.append(Filter(_S(_COMP, _EQ, _S(_CONS, rsel, ssel))))
    parts.append(_DISTL)
    join_one = _S(_COMP, *parts)
    return _S(_COMP, _flatten, _S(_ALPHA, join_one), _DISTR)


def Restrict(cols_L, cols_M):
    """Codd restriction R_{L|M}S (§2.1.5): the maximal R'⊆R with pi_L(R')=pi_M(S),
    over ⟨R, S⟩ — the semijoin keeping rows of R whose L-key occurs in pi_M(S).
    Toolchain remainder, composing canon pieces host-side.
        Restrict(L,M) = α(1) ∘ Filter(pi_L(r) ∈ pi_M(S)) ∘ distr ∘ [1, pi_M∘2]
    """
    rowL = _S(_CONS, *tuple(A(i) for i in cols_L))
    inMS = _S(_COMP, _member, _S(_CONS, _S(_COMP, rowL, _1), _2))
    pair = _S(_CONS, _1, _S(_COMP, Project(cols_M), _2))
    return _S(_COMP, _S(_ALPHA, _1), Filter(inMS), _DISTR, pair)
