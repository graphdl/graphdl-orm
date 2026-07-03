"""Codd's adequate collection theta1 (Codd §2.2) — the Python BINDING of the
canonical definitions in shared/theta.py (intersection source, one file for every
host). The closed objects here ARE the canon values, loaded from the shared file;
the parameterized constructors APPLY the canonical builders through the reducer
(canon boots with the package, so the names resolve). Nothing is defined twice:
this module binds, it does not author. "Navigation needs no separate query language" —
each operator is a rho-application over the population P.
"""
from . import canon as _canon
from .lam import atom as A, to_lam
from .reduce import apply as _apply

_C = dict(_canon.read("theta.py"))                           # the shared file, verbatim

member = _C["theta:member"]
dedup = _C["theta:dedup"]
flatten = _C["theta:flatten"]
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
    Empty `pairs` is the degenerate cross product; empty `keep` is the semijoin. The
    canonical COND-over-null builder applied to ⟨pairs, keep⟩ (shared/theta.py).
        match   = eq∘[⟨ri…⟩∘1, ⟨si…⟩∘2]
        combine = cat∘[1, ⟨keep…⟩∘2]        (just 1 when keep is empty)
        R⋈S     = flatten ∘ α( α(combine) ∘ Filter(match) ∘ distl ) ∘ distr
    """
    return _apply(A("theta:JoinOn"),
                  to_lam((tuple(tuple(p) for p in pairs), tuple(keep))))


def Restrict(cols_L, cols_M):
    """Codd restriction R_{L|M}S (§2.1.5): the maximal R'⊆R with pi_L(R')=pi_M(S),
    over ⟨R, S⟩ — the semijoin keeping rows of R whose L-key occurs in pi_M(S). The
    canonical builder applied to ⟨L, M⟩ (shared/theta.py).
        Restrict(L,M) = α(1) ∘ Filter(pi_L(r) ∈ pi_M(S)) ∘ distr ∘ [1, pi_M∘2]
    """
    return _apply(A("theta:Restrict"),
                  to_lam((tuple(cols_L), tuple(cols_M))))
