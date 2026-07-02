"""Constraint families as FFP violation expressions: (rho c) : P = V_c (Def. Violation).

A constraint is *implemented* as an FFP object c; rho (define + the reducer) *reflects*
it back to the ORM layer, where (rho c) : P is the set of population tuples that violate
it. The meta-object 'the constraint' and its FFP implementation are the same entity under
rho — the metamodel describes it, FFP implements it, rho reflects it back. Each family is
authored in Codd theta1 + the Backus base and reduced by the one mu; nothing is host code.
"""
from . import lam as L
from .lam import atom as A, to_lam
from . import theta as T
from .defs import define
from .reduce import apply

def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)

_COMP, _CONS, _ALPHA, _ID, _CONST = A("COMP"), A("CONS"), A("ALPHA"), A("id"), A("CONST")
_EQ, _NOT, _NULL, _AND, _OR, _DISTL, _DISTR = A("eq"), A("not"), A("null"), A("and"), A("or"), A("distl"), A("distr")
_LT, _GT = A("lt"), A("gt")
_1, _2 = A(1), A(2)


def _key(roles):
    """[sel_i1 … sel_ik] : t -> the tuple's key over the constrained roles."""
    return _S(_CONS, *tuple(A(i) for i in roles))


def uniqueness(roles):
    """Uniqueness constraint over `roles` (ORM's fundamental constraint), as the FFP
    violation object c with (rho c) : P = the tuples of P sharing their key with a
    *different* tuple:
        hasDup:⟨t,P⟩ = not null ( Filter(key(1)=key(2) ∧ 1≠2) : (distl:⟨t,P⟩) )
        V_uc         = α(1) ∘ Filter(hasDup) ∘ distr ∘ [id, id]
    """
    key = _key(roles)
    same = _S(_COMP, _EQ, _S(_CONS, _S(_COMP, key, A(1)), _S(_COMP, key, A(2))))   # key(t)=key(s)
    diff = _S(_COMP, _NOT, _EQ)                                                     # t ≠ s
    both = _S(_COMP, _AND, _S(_CONS, same, diff))
    has_dup = _S(_COMP, _NOT, _NULL, T.Filter(both), _DISTL)                        # ⟨t,P⟩ -> T/F
    return _S(_COMP, _S(_ALPHA, A(1)), T.Filter(has_dup), _DISTR, _S(_CONS, _ID, _ID))


def ring_irreflexive(roles=(1, 2)):
    """Irreflexive ring on a binary fact type: no x relates to itself. V = the ⟨x,x⟩ facts."""
    r1, r2 = A(roles[0]), A(roles[1])
    return T.Filter(_S(_COMP, _EQ, _S(_CONS, r1, r2)))


def ring_symmetric(roles=(1, 2)):
    """Symmetric ring: x R y ⟹ y R x. V = the ⟨x,y⟩ (x≠y) whose reverse ⟨y,x⟩ is absent from P.
        α(1) ∘ Filter(x≠y ∧ ⟨y,x⟩∉P) ∘ distr ∘ [id, id]"""
    r1, r2 = A(roles[0]), A(roles[1])
    fx, fy = _S(_COMP, r1, _1), _S(_COMP, r2, _1)            # x, y of the fact (1:pair)
    swap = _S(_CONS, fy, fx)                                 # ⟨y, x⟩
    swap_absent = _S(_COMP, _NOT, T.member, _S(_CONS, swap, _2))   # ⟨y,x⟩ ∉ P
    neq = _S(_COMP, _NOT, _EQ, _S(_CONS, fx, fy))            # x ≠ y
    viol = _S(_COMP, _AND, _S(_CONS, neq, swap_absent))
    return _S(_COMP, _S(_ALPHA, _1), T.Filter(viol), _DISTR, _S(_CONS, _ID, _ID))


def value_range(role, lo, hi):
    """Value constraint: the value at `role` lies in [lo, hi]. V = the facts outside the range
    (a boundary comparison on the ORM-typed value)."""
    rv = A(role)
    below = _S(_COMP, _LT, _S(_CONS, rv, _S(_CONST, to_lam(lo))))
    above = _S(_COMP, _GT, _S(_CONS, rv, _S(_CONST, to_lam(hi))))
    return T.Filter(_S(_COMP, _OR, _S(_CONS, below, above)))


def mandatory():
    """Simple mandatory role: every entity plays the role. Input ⟨entities, players⟩;
    V = entities ∖ players (Codd setminus) — the entities that play no fact."""
    return T.setminus


def subset():
    """Subset constraint A ⊆ B. Input ⟨A, B⟩; V = A ∖ B — the A-facts not present in B."""
    return T.setminus


def violations(constraint_obj, population):
    """(rho c) : P — reduce the violation object against a population (an FFP object)."""
    return apply(constraint_obj, population)


def register_constraint(name, constraint_obj):
    """Reflect a constraint into the ORM layer under `name`: define(name,c) makes rho(name)
    denote (rho c), so apply(atom(name), P) = V_c. The meta-object IS the reflected FFP."""
    define(name, constraint_obj)
