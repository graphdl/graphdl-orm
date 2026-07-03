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


def uniqueness(roles):
    """Uniqueness constraint over `roles` (ORM's fundamental constraint): the
    canonical builder (shared/constraints.py) applied to the key roles.
        hasDup:⟨t,P⟩ = not null ( Filter(key(1)=key(2) ∧ 1≠2) : (distl:⟨t,P⟩) )
        V_uc         = α(1) ∘ Filter(hasDup) ∘ distr ∘ [id, id]
    """
    from .reduce import apply as _apply
    return _apply(A("constraints:uniqueness"), to_lam(tuple(roles)))


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


def ring_asymmetric(roles=(1, 2)):
    """Asymmetric ring (Halpin §7.3): xRy → ¬yRx, with x and y not necessarily distinct,
    so reflexive pairs violate too (asymmetric = antisymmetric + irreflexive). V = the
    facts whose swap is also in P."""
    r1, r2 = A(roles[0]), A(roles[1])
    fx, fy = _S(_COMP, r1, _1), _S(_COMP, r2, _1)
    swap = _S(_CONS, fy, fx)
    viol = _S(_COMP, T.member, _S(_CONS, swap, _2))
    return _S(_COMP, _S(_ALPHA, _1), T.Filter(viol), _DISTR, _S(_CONS, _ID, _ID))


def ring_antisymmetric(roles=(1, 2)):
    """Antisymmetric ring (§7.3): x ≠ y & xRy → ¬yRx. Reflexive pairs are allowed."""
    r1, r2 = A(roles[0]), A(roles[1])
    fx, fy = _S(_COMP, r1, _1), _S(_COMP, r2, _1)
    swap_in = _S(_COMP, T.member, _S(_CONS, _S(_CONS, fy, fx), _2))
    neq = _S(_COMP, _NOT, _EQ, _S(_CONS, fx, fy))
    viol = _S(_COMP, _AND, _S(_CONS, neq, swap_in))
    return _S(_COMP, _S(_ALPHA, _1), T.Filter(viol), _DISTR, _S(_CONS, _ID, _ID))


def ring_intransitive(roles=(1, 2)):
    """Intransitive ring (§7.3): xRy & yRz → ¬xRz. V = the facts of P that complete a
    two-step chain, i.e. P ∩ π₁₃(P ⋈ P)."""
    chains = _S(_COMP, T.Project([1, 3]), T.NatJoin(2), _S(_CONS, _ID, _ID))
    in_chains = _S(_COMP, T.member, _S(_CONS, _1, _2))       # ⟨t, chains⟩ → t ∈ chains
    return _S(_COMP, _S(_ALPHA, _1), T.Filter(in_chains), _DISTR, _S(_CONS, _ID, chains))


def ring_acyclic(roles=(1, 2)):
    """Acyclic ring (§7.3): "no path via the relation from an object back to itself".
    V = the reflexive pairs of the transitive closure, the closure computed by the same
    derive lfp that serves derivation rules (per Mapping ORM to Datalog)."""
    from . import system as _sys
    tc = _sys.derive_of([_sys.join_rule(2, [1, 3])])
    return _S(_COMP, T.Filter(_S(_COMP, _EQ, _S(_CONS, _1, _2))), tc)


def frequency(roles, lo=None, hi=None):
    """Occurrence frequency (§7.2): "each member of pop(roles) occurs there exactly n
    times", generalized to [lo, hi]; a local constraint on the role population, not the
    object type, so unplayed members are fine. V = the facts whose key count is out of
    bounds. The canonical builder applied to ⟨roles, lo?, hi?⟩ (an absent bound is the
    empty sequence)."""
    from .reduce import apply as _apply
    from .lam import to_lam as _tl
    return _apply(A("constraints:frequency"),
                  _tl((tuple(roles),
                       (lo,) if lo is not None else (),
                       (hi,) if hi is not None else ())))


def value_range(role, lo=None, hi=None, lo_open=False, hi_open=False):
    """Value constraint over a continuous range (NORMA's value ranges). The value at
    `role` must lie in the range with bounds lo/hi (None = unbounded, *_open =
    exclusive). V = the facts outside it. The canonical builder applied to
    ⟨role, ⟨lo, openflag⟩?, ⟨hi, openflag⟩?⟩."""
    from .reduce import apply as _apply
    from .lam import to_lam as _tl
    return _apply(A("constraints:value_range"),
                  _tl((role,
                       (lo, "T" if lo_open else "F") if lo is not None else (),
                       (hi, "T" if hi_open else "F") if hi is not None else ())))


def value_enumeration(role, values):
    """Value constraint over an enumeration (NORMA's 'the possible values of X are
    …'). The value at `role` must be one of `values`. V = the facts whose value is
    not in the set. The canonical builder applied to ⟨role, values⟩."""
    from .reduce import apply as _apply
    from .lam import to_lam as _tl
    return _apply(A("constraints:value_enumeration"),
                  _S(A(role), _tl(tuple(values))))


_CC = None


def _canon_c(name):
    global _CC
    if _CC is None:
        from . import canon as _canon
        _CC = dict(_canon.read("constraints.py"))
    return _CC[name]


def mandatory():
    """Simple mandatory role: every entity plays the role. Input ⟨entities, players⟩;
    V = entities ∖ players (Codd setminus) — the entities that play no fact. The
    canon value (shared/constraints.py)."""
    return _canon_c("constraints:mandatory")


def subset():
    """Subset constraint A ⊆ B (NORMA 'if A then B' — implication by modus ponens). Input ⟨A, B⟩;
    V = A ∖ B — the antecedent facts whose consequent does not hold. The canon value."""
    return _canon_c("constraints:subset")


def equality():
    """Equality constraint A = B (NORMA 'A if and only if B'). Input ⟨A, B⟩; V = (A ∖ B) ∪ (B ∖ A),
    the symmetric difference — facts on one side without their counterpart on the other. The
    canon value."""
    return _canon_c("constraints:equality")


# --- set-comparison over a participation population ⟨⟨entity, clause⟩ …⟩ (one fact per clause the
# entity participates in). These reduce to theta1 constraints already defined. ---
_CAT = A("cat")


def exclusion():
    """Exclusion — at most one of the clauses holds per entity. V = the participations whose entity
    also appears with a DIFFERENT clause (uniqueness on the entity role, applied through the
    apply primitive in the canon)."""
    return _canon_c("constraints:exclusion")


def inclusive_or():
    """Inclusive-or / disjunctive mandatory — at least one clause holds per entity. Input
    ⟨universe, players⟩ (players = entities in some clause); V = universe ∖ players (setminus).
    The canon value."""
    return _canon_c("constraints:inclusive_or")


def exclusive_or():
    """Exclusive-or — exactly one clause holds per entity. Input ⟨universe, participation⟩; V = the
    entities in NO clause (universe ∖ pi1(participation)) together with those in TWO OR MORE (the
    uniqueness violations of the participation) — everyone not holding exactly one. The canon
    value."""
    return _canon_c("constraints:exclusive_or")


# ============================ scoped (cross-cell) families ====================
# A scoped violation expression consumes ⟨P, D⟩: P is the TARGET cell's post-derive
# population (the cell this commit writes), and every sibling population is fetched from
# the frozen D — validate runs before the commit, so the target cell's copy in D is stale
# and must come from P, while sibling cells are untouched by this step (Def. iso).

def _pop_of(cell_name):
    """⟨P,D⟩ → the population of a SIBLING cell, fetched from the frozen D. A non-string
    argument is taken as a ready population EXPRESSION over D (the RMAP view seam:
    an absorbed fact type's population reassembled through the index), composed the
    same way."""
    from . import ast
    src = ast.FetchPop(cell_name) if isinstance(cell_name, str) else cell_name
    return _S(_COMP, src, _2)


_P = _1                                                       # ⟨P,D⟩ → the target population


def _scoped(name, cell, host):
    """A string cell name applies the canonical builder; a ready population
    EXPRESSION (the RMAP view seam) keeps the host composition until system.py
    migrates."""
    if isinstance(cell, str):
        from .reduce import apply as _apply
        return _apply(A(name), A(cell))
    return host()


def scoped_mandatory_entities(entity_cell):
    """Mandatory, attached to the FACT-TYPE cell (P = the fact population): the instances
    in the entity type's own cell that play no fact. V = π1(entities) ∖ π1(P).
    Canonical for a named sibling."""
    def host():
        ents = _S(_COMP, T.Project([1]), _pop_of(entity_cell))
        players = _S(_COMP, T.Project([1]), _P)
        return _S(_COMP, T.setminus, _S(_CONS, ents, players))
    return _scoped("constraints:scoped_mandatory_entities", entity_cell, host)


def scoped_mandatory_facts(ft_cell):
    """Mandatory, attached to the ENTITY cell (P = the entity population): the entities of
    P that play no fact in the fact-type cell. V = π1(P) ∖ π1(↑ft). Canonical for a
    named sibling."""
    def host():
        ents = _S(_COMP, T.Project([1]), _P)
        players = _S(_COMP, T.Project([1]), _pop_of(ft_cell))
        return _S(_COMP, T.setminus, _S(_CONS, ents, players))
    return _scoped("constraints:scoped_mandatory_facts", ft_cell, host)


def scoped_subset(consequent_cell):
    """Subset A ⊆ B, attached to the antecedent cell (P = A): V = P ∖ ↑B, tuple-wise —
    the clause readings resolve to fact types whose role order matches (modus ponens).
    Canonical for a named sibling."""
    def host():
        return _S(_COMP, T.setminus, _S(_CONS, _P, _pop_of(consequent_cell)))
    return _scoped("constraints:scoped_subset", consequent_cell, host)


def scoped_equality_side(other_cell):
    """Equality A = B, attached to ONE side (P = this side): the symmetric difference
    (P ∖ ↑other) ∪ (↑other ∖ P). Canonical for a named sibling."""
    def host():
        ab = _S(_COMP, T.setminus, _S(_CONS, _P, _pop_of(other_cell)))
        ba = _S(_COMP, T.setminus, _S(_CONS, _pop_of(other_cell), _P))
        return _S(_COMP, _CAT, _S(_CONS, ab, ba))
    return _scoped("constraints:scoped_equality_side", other_cell, host)


def value_comparison(op, col, lit):
    """The value-comparison family expression (paper Def. Schema; NORMA
    ValueComparisonConstraint). RESERVED for the canonical role-vs-role verbalization
    when it lands; a LITERAL bound is canonically a VALUE CONSTRAINT range ('The
    possible values of X are at most 5.'), already wired — no non-canonical FORML."""
    return T.Filter(_S(_COMP, A("not"), A(op), _S(_CONS, A(col), _S(_CONST, A(lit)))))


def _participation(clause_fts, target_ft, pops=None):
    """⟨P,D⟩ → ⟨⟨entity, clause⟩ …⟩ over ALL clause cells: the target clause reads from P,
    the sibling clauses from D. Each row is tagged with its clause's fact-type id. `pops`
    overrides a clause's population with an expression over D (the RMAP view seam)."""
    parts = []
    for ft in clause_fts:
        src = _P if ft == target_ft else _pop_of((pops or {}).get(ft, ft))
        tag = _S(_ALPHA, _S(_CONS, _1, _S(_CONST, A(ft))))    # row → ⟨entity, clause⟩
        parts.append(_S(_COMP, tag, src))
    return _S(_COMP, T.flatten, _S(_CONS, *parts))


def scoped_exclusion(clause_fts, target_ft, pops=None):
    """Exclusion over clause fact types, attached to `target_ft`'s cell: at most one clause
    per entity — uniqueness on the entity role of the participation. Canonical unless a
    pops override (the view seam) rides along."""
    if not pops:
        from .reduce import apply as _apply
        from .lam import to_lam as _tl
        return _apply(A("constraints:scoped_exclusion"),
                      _S(_tl(tuple(clause_fts)), A(target_ft)))
    return _S(_COMP, exclusion(), _participation(clause_fts, target_ft, pops))


def scoped_exclusive_or(subject_cell, clause_fts, target_ft, pops=None):
    """Exactly one clause per entity: exclusive_or over ⟨universe, participation⟩, the
    universe being the subject type's own instance cell. Canonical unless a pops
    override rides along."""
    if not pops and isinstance(subject_cell, str):
        from .reduce import apply as _apply
        from .lam import to_lam as _tl
        return _apply(A("constraints:scoped_exclusive_or"),
                      _S(A(subject_cell), _tl(tuple(clause_fts)), A(target_ft)))
    pair = _S(_CONS, _pop_of(subject_cell), _participation(clause_fts, target_ft, pops))
    return _S(_COMP, exclusive_or(), pair)


def scoped_external_uniqueness(other_ft, cols):
    """External uniqueness over two fact types (Halpin §10.3, Fig. 10.21 verbatim:
    "equivalent to an internal uniqueness constraint spanning [the columns] in the
    natural join of the two tables"). ⟨P, D⟩: join the target population with the
    sibling cell on the shared key (role 1 = role 1), then the internal UC over `cols`
    of the joined tuples. Canonical for a named sibling."""
    if isinstance(other_ft, str):
        from .reduce import apply as _apply
        from .lam import to_lam as _tl
        return _apply(A("constraints:scoped_external_uniqueness"),
                      _S(A(other_ft), _tl(tuple(cols))))
    join = _S(_COMP, T.NatJoin(1), _S(_CONS, _P, _pop_of(other_ft)))
    return _S(_COMP, uniqueness(cols), join)


def scoped_inclusive_or(subject_cell, clause_fts, target_ft, pops=None):
    """At least one clause per entity (disjunctive mandatory): universe ∖ players.
    Canonical unless a pops override rides along."""
    if not pops and isinstance(subject_cell, str):
        from .reduce import apply as _apply
        from .lam import to_lam as _tl
        return _apply(A("constraints:scoped_inclusive_or"),
                      _S(A(subject_cell), _tl(tuple(clause_fts)), A(target_ft)))
    players = _S(_COMP, T.Project([1]), _participation(clause_fts, target_ft, pops))
    pair = _S(_CONS, _S(_COMP, T.Project([1]), _pop_of(subject_cell)), players)
    return _S(_COMP, T.setminus, pair)


def violations(constraint_obj, population):
    """(rho c) : P — reduce the violation object against a population (an FFP object)."""
    return apply(constraint_obj, population)


def register_constraint(name, constraint_obj):
    """Reflect a constraint into the ORM layer under `name`: define(name,c) makes rho(name)
    denote (rho c), so apply(atom(name), P) = V_c. The meta-object IS the reflected FFP."""
    define(name, constraint_obj)
