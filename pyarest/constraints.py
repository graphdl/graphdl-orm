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


_LE, _GE = A("le"), A("ge")


def value_range(role, lo=None, hi=None, lo_open=False, hi_open=False):
    """Value constraint over a continuous range (NORMA's value ranges). The value at `role` must
    lie in the range with bounds lo/hi (None = unbounded, *_open = exclusive). V = the facts
    outside it — a boundary comparison on the ORM-typed value. Closed [lo,hi] violates below lo
    or above hi; open bounds use le/ge instead of lt/gt."""
    rv = A(role)
    parts = []
    if lo is not None:
        parts.append(_S(_COMP, _LE if lo_open else _LT, _S(_CONS, rv, _S(_CONST, to_lam(lo)))))
    if hi is not None:
        parts.append(_S(_COMP, _GE if hi_open else _GT, _S(_CONS, rv, _S(_CONST, to_lam(hi)))))
    if len(parts) == 2:
        pred = _S(_COMP, _OR, _S(_CONS, parts[0], parts[1]))
    elif parts:
        pred = parts[0]
    else:
        pred = _S(_CONST, A("F"))                             # unbounded both ways ⇒ nothing violates
    return T.Filter(pred)


def value_enumeration(role, values):
    """Value constraint over an enumeration (NORMA's 'the possible values of X are …'). The value
    at `role` must be one of `values`. V = the facts whose value is not in the set (via member)."""
    allowed = to_lam(tuple(values))
    in_set = _S(_COMP, T.member, _S(_CONS, A(role), _S(_CONST, allowed)))   # value ∈ allowed
    return T.Filter(_S(_COMP, _NOT, in_set))                                # keep facts NOT in the set


def mandatory():
    """Simple mandatory role: every entity plays the role. Input ⟨entities, players⟩;
    V = entities ∖ players (Codd setminus) — the entities that play no fact."""
    return T.setminus


def subset():
    """Subset constraint A ⊆ B (NORMA 'if A then B' — implication by modus ponens). Input ⟨A, B⟩;
    V = A ∖ B — the antecedent facts whose consequent does not hold."""
    return T.setminus


def equality():
    """Equality constraint A = B (NORMA 'A if and only if B'). Input ⟨A, B⟩; V = (A ∖ B) ∪ (B ∖ A),
    the symmetric difference — facts on one side without their counterpart on the other."""
    ab = _S(_COMP, T.setminus, _S(_CONS, _1, _2))
    ba = _S(_COMP, T.setminus, _S(_CONS, _2, _1))
    return _S(_COMP, _CAT, _S(_CONS, ab, ba))


# --- set-comparison over a participation population ⟨⟨entity, clause⟩ …⟩ (one fact per clause the
# entity participates in). These reduce to theta1 constraints already defined. ---
_CAT = A("cat")


def exclusion():
    """Exclusion — at most one of the clauses holds per entity. V = the participations whose entity
    also appears with a DIFFERENT clause (uniqueness on the entity role of the participation)."""
    return uniqueness([1])


def inclusive_or():
    """Inclusive-or / disjunctive mandatory — at least one clause holds per entity. Input
    ⟨universe, players⟩ (players = entities in some clause); V = universe ∖ players (setminus)."""
    return T.setminus


def exclusive_or():
    """Exclusive-or — exactly one clause holds per entity. Input ⟨universe, participation⟩; V = the
    entities in NO clause (universe ∖ pi1(participation)) together with those in TWO OR MORE (the
    uniqueness violations of the participation) — everyone not holding exactly one."""
    players = _S(_COMP, T.Project([1]), _2)                       # entities that participate
    none = _S(_COMP, T.setminus, _S(_CONS, _1, players))          # in no clause
    many = _S(_COMP, T.Project([1]), uniqueness([1]), _2)         # in >= 2 clauses
    return _S(_COMP, _CAT, _S(_CONS, none, many))                 # none ∪ many


def violations(constraint_obj, population):
    """(rho c) : P — reduce the violation object against a population (an FFP object)."""
    return apply(constraint_obj, population)


def register_constraint(name, constraint_obj):
    """Reflect a constraint into the ORM layer under `name`: define(name,c) makes rho(name)
    denote (rho c), so apply(atom(name), P) = V_c. The meta-object IS the reflected FFP."""
    define(name, constraint_obj)
