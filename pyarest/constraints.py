"""Constraint families as FFP violation expressions: (rho c) : P = V_c (Def. Violation).

A constraint is *implemented* as an FFP object c; rho (define + the reducer) *reflects*
it back to the ORM layer, where (rho c) : P is the set of population tuples that violate
it. The meta-object 'the constraint' and its FFP implementation are the same entity under
rho — the metamodel describes it, FFP implements it, rho reflects it back. Each family is
authored in Codd theta1 + the Backus base and reduced by the one mu; nothing is host code.
"""
from . import lam as L
from .lam import atom as A
from . import theta as T
from .defs import define
from .reduce import apply

def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)

_COMP, _CONS, _ALPHA, _ID = A("COMP"), A("CONS"), A("ALPHA"), A("id")
_EQ, _NOT, _NULL, _AND, _DISTL, _DISTR = A("eq"), A("not"), A("null"), A("and"), A("distl"), A("distr")


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


def violations(constraint_obj, population):
    """(rho c) : P — reduce the violation object against a population (an FFP object)."""
    return apply(constraint_obj, population)


def register_constraint(name, constraint_obj):
    """Reflect a constraint into the ORM layer under `name`: define(name,c) makes rho(name)
    denote (rho c), so apply(atom(name), P) = V_c. The meta-object IS the reflected FFP."""
    define(name, constraint_obj)
