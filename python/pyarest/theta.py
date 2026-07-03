"""Codd's adequate collection theta1 (Codd §2.2) as FFP objects over a population.

Projection, natural join, tie, and restriction — each a compiled FFP object in
Backus's combining forms, reduced by the one mu. No raw lambda here and no host
logic (spec D4): theta1 is authored *in* the Backus base. "Navigation needs no
separate query language" — each operator is a rho-application over the population P.

Codd's *restriction* (§2.1.5) is the binary R_{L|M}S (`Restrict`); the retrieval the
whitepaper writes "Filter(p):X" is Codd's *selection* sigma (a unary predicate filter)
which generalizes it and is what the constraint evaluators reduce to.
"""
from . import lam as L
from .lam import atom as A, PHI

def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)

_COMP, _COND, _CONS, _CONST = A("COMP"), A("COND"), A("CONS"), A("CONST")
_INSERT, _ALPHA = A("INSERT"), A("ALPHA")
_APNDL, _APNDR, _ID = A("apndl"), A("apndr"), A("id")
_EQ, _NULL, _NOT, _DISTL, _DISTR = A("eq"), A("null"), A("not"), A("distl"), A("distr")
_TL, _CAT, _1R, _TLR = A("tl"), A("cat"), A("1r"), A("tlr")
_1, _2 = A(1), A(2)                                          # selectors are numeric atoms

# apndr∘[id, φ̄] : X → ⟨x1..xn, φ⟩ — seed a right-fold over X with an empty accumulator
_append_phi = _S(_COMP, _APNDR, _S(_CONS, _ID, _S(_CONST, PHI)))


def Filter(p):
    """Codd selection sigma_p as the FFP object Filter(p): keep xi where p:xi = T.
        keep     = (p∘1 → apndl ; 2)
        Filter p = (/keep) ∘ apndr∘[id, φ̄]
    """
    keep = _S(_COND, _S(_COMP, p, _1), _APNDL, _2)
    return _S(_COMP, _S(_INSERT, keep), _append_phi)


# member:⟨x, acc⟩ = T iff x ∈ acc   (not ∘ null ∘ Filter(eq) ∘ distl)
_member = member = _S(_COMP, _NOT, _NULL, Filter(_EQ), _DISTL)
# dedup rows (second half of Codd projection, §2.1.2): (/(member→2;apndl)) ∘ apndr∘[id,φ̄]
_dedup = dedup = _S(_COMP, _S(_INSERT, _S(_COND, _member, _2, _APNDL)), _append_phi)
# flatten a sequence of sequences into one — concat seeded with φ
_flatten = flatten = _S(_COMP, _S(_INSERT, _CAT), _append_phi)
# setminus:⟨A,B⟩ = the elements of A not in B — α(1) ∘ Filter(¬member) ∘ distr.
# (the semi-naive fixpoint test: derive stops when F_S(P) ∖ P = φ.)
setminus = _S(_COMP, _S(_ALPHA, _1), Filter(_S(_COMP, _NOT, _member)), _DISTR)


def Project(cols):
    """Codd projection pi_L (§2.1.2): keep roles `cols` (1-based) from each tuple,
    then remove duplicate rows.  pi_L = dedup ∘ α[sel_c1,…,sel_ck]."""
    row = _S(_CONS, *tuple(A(i) for i in cols))
    return _S(_COMP, _dedup, _S(_ALPHA, row))


def NatJoin(i):
    """Codd natural join R*S (§2.1.3), joining R.i = S.1 (permute to arrange).
    Unambiguous when the joined role is functional (ORM uniqueness) — Codd's only join.
        match=eq∘[i∘1, 1∘2] ; combine=cat∘[1, tl∘2]
        R*S = flatten ∘ α( α(combine) ∘ Filter(match) ∘ distl ) ∘ distr
    """
    si = A(i)
    match = _S(_COMP, _EQ, _S(_CONS, _S(_COMP, si, _1), _S(_COMP, _1, _2)))
    combine = _S(_COMP, _CAT, _S(_CONS, _1, _S(_COMP, _TL, _2)))
    join_one = _S(_COMP, _S(_ALPHA, combine), Filter(match), _DISTL)
    return _S(_COMP, _flatten, _S(_ALPHA, join_one), _DISTR)


def JoinOn(pairs, keep):
    """Codd's join (§2.1.3) in its general equi form: R ⋈ S on {R.ri = S.si} for the
    (ri, si) in `pairs`, emitting r ++ s[keep] (the fresh columns, in clause order).
    Empty `pairs` is the degenerate cross product; empty `keep` is the semijoin. The
    SAME primitives as NatJoin — eq over selector tuples — so every carrier runs it
    unchanged; NatJoin(i) is the one-column case JoinOn(((i,1),), (2..w)).
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


# Codd tie gamma (§2.1.3): degree n → n-1, keep tuples with first = last, drop last
Tie = _S(_COMP, _S(_ALPHA, _TLR), Filter(_S(_COMP, _EQ, _S(_CONS, _1, _1R))))


def Restrict(cols_L, cols_M):
    """Codd restriction R_{L|M}S (§2.1.5): the maximal R'⊆R with pi_L(R')=pi_M(S),
    over ⟨R, S⟩ — the semijoin keeping rows of R whose L-key occurs in pi_M(S).
        Restrict(L,M) = α(1) ∘ Filter(pi_L(r) ∈ pi_M(S)) ∘ distr ∘ [1, pi_M∘2]
    """
    rowL = _S(_CONS, *tuple(A(i) for i in cols_L))
    inMS = _S(_COMP, _member, _S(_CONS, _S(_COMP, rowL, _1), _2))
    pair = _S(_CONS, _1, _S(_COMP, Project(cols_M), _2))
    return _S(_COMP, _S(_ALPHA, _1), Filter(inMS), _DISTR, pair)
