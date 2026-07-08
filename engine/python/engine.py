"""The engine in ONE file (the seven-file shape): ast (cells, the store
walk, the AST transition, DefineIn), constraints (the violation expressions,
local and scoped), and system (the create pipeline, derive to the least
fixed point with the joint strata, HATEOAS, machines as values). Each
section keeps its docstring; the package init aliases the old names, and
lazy in-body imports resolve through those aliases at call time."""

# ===================== ast: cells and the store =====================
"""The AST layer (Backus §14): the state D is a sequence of cells; a command runs as the
single transition create_cell:⟨input, D⟩ = ⟨output, D'⟩ over ONE entity's cell (cell
isolation — distinct entities' handlers write disjoint cells and never interfere). The
representation is o = ⟨P'', V⟩; the state commits P'' back to the cell iff V carries no
alethic violation (Def. Violation / completeness of state transfer). Cells and fetch ↑ /
store ↓ are Backus's (§13.3.4); everything in the transition is an FFP object reduced by mu.
"""
from . import lam as L
from .lam import atom as A, PHI
from .reduce import apply

def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)

CELL = A("CELL")
DEFAULT = A("#")                                             # ↑ of an absent cell (Backus §13.3.4)
_COMP, _CONS, _CONST, _COND = A("COMP"), A("CONS"), A("CONST"), A("COND")
_1, _2, _3 = A(1), A(2), A(3)
_APNDL, _NULL, _NOT, _EQ, _APPLY, _DISTR = A("apndl"), A("null"), A("not"), A("eq"), A("apply"), A("distr")
# a validate that always commits: ⟨P,D⟩ → ⟨P, φ, F⟩ — empty violations, alethic flag false


def cell(name, contents):
    """A cell ⟨CELL, name, contents⟩ (Backus §13.3.4)."""
    return _S(CELL, A(name), contents)


def Fetch(name):
    """↑name — contents (role 3) of the first cell named `name`, else # (Backus
    §13.3.4). The canonical builder applied to the name (shared/ast.canon)."""
    return apply(A("ast:Fetch"), A(name))


def FetchPop(name):
    """The create pipeline's view of a cell as a POPULATION: ↑name, with an absent
    cell an empty population — the fresh-cell default is the pipeline's explicit
    choice (a COND on #), never a change to ↑'s meaning. Canonical."""
    return apply(A("ast:FetchPop"), A(name))


def Pop(name):
    """(pop n) — remove the FIRST cell named `name`, preserving deeper ones (§13.3.4:
    cells of one name form a LIFO stack). Canonical (a WHILE-fold over
    ⟨removed?, acc, rest⟩ standing in for Backus's recursive definition)."""
    return apply(A("ast:Pop"), A(name))


def Purge(name):
    """(purge n) — remove ALL cells named `name` (§13.3.4's other operator).
    Canonical."""
    return apply(A("ast:Purge"), A(name))


def Store(name):
    """↓name — ⟨x, D⟩ → (push n):⟨x, (pop n):D⟩ (§13.3.4 verbatim): replace the TOP
    of the stack named `name`; deeper same-named cells survive. Canonical."""
    return apply(A("ast:Store"), A(name))


def DefineIn(name, obj):
    """D → D′ with the definition stored as an ORDINARY cell ⟨CELL, name, obj⟩ of D
    by ↓name (Backus §13.3.5: such a cell has the same effect as Def name ≡ ρobj).
    Definitions travel with the store (Prop. tenant / Cor. closure). Canonical,
    applied to ⟨name, obj⟩."""
    return apply(A("ast:DefineIn"), _S(A(name), obj))


def build_system(validate_obj=None, cell_name="FILE", resolve_obj=None, derive_obj=None, links_obj=None,
                 machine=None, mealy_obj=None, index_cell=None, append_cell=None):
    """The transition create_cell:⟨I, D⟩ → ⟨⟨P'',V⟩, D'⟩ over one cell, wired with a schema's
    validate (and optionally its resolve/derive). It touches only `cell_name` — plus, when
    `machine=((table, col, width), sm_obj)` is wired, the governed entities' status COLUMN
    (status(e) is the "is currently in Status" fact type, absorbed by RMAP onto the object
    type's rows): the trigger fact entering P advances the machine within the SAME step
    (Prop. onestep), atomically with the commit, through the authorized row_overwrite.
    With `mealy_obj` (same input shape as sm_obj) the fired transitions' Mealy emissions are
    appended to the representation o as its last part. With `index_cell` (the routed-write
    case) the table's key index records I's key in the SAME commit chain, so refusal leaves
    the index untouched and re-writes stay deduplicated. Commits iff the alethic flag is
    false."""
    from .lam import to_lam

    def slot(v):
        return to_lam(()) if v is None else _S(v)

    m = to_lam(()) if machine is None else _S(
        to_lam(machine[0]), machine[1], *(A(r) for r in machine[2:]))
    record = _S(A(cell_name), slot(validate_obj), slot(resolve_obj), slot(derive_obj),
                slot(links_obj), m, slot(mealy_obj),
                slot(A(index_cell)) if index_cell is not None else to_lam(()),
                slot(A(append_cell)) if append_cell is not None else to_lam(()))
    return apply(A("ast:build_system"), record)


def step_input(x, D, fuel=None):
    """The plain transition on an input: μ(SYSTEM:x) under D's own definitions, the
    transition rules applied (§14.3.1), optionally fuel-supervised."""
    from . import defs
    with defs.step(D, fuel):
        return _transition(apply(A("SYSTEM"), x), D)


def reset(x, D, fuel=None):
    """§14.3.2 verbatim: the system accepts ⟨RESET, x⟩ at any time. (a) If SYSTEM is
    defined in the current state D, it 'aborts its current computation without altering
    D' and treats x as a new normal input. (b) If SYSTEM is not defined, x is appended
    to D as its first element — the bootstrap of §14.4.3."""
    from . import defs
    if defs._cells_of(D).get("SYSTEM") is not None:
        return step_input(x, D, fuel)
    return L.SEQ(L.CONS(x)(L._list(D)))


def _cell_value(D, name):
    from . import defs as _d
    return _d._cells_of(D).get(name)


def _component_step(x, Dc, fuel):
    """One component transition for the framework forms: a store with no SYSTEM answers
    ⟨ERROR, unchanged⟩ up front (the same check §14.3.2's RESET makes) — an unresolved
    SYSTEM would otherwise be ⊥, which is divergence, and the framework layer is exactly
    where Backus puts that answer (§14.3.1)."""
    from . import defs as _d
    if _d._cells_of(Dc).get("SYSTEM") is None:
        from .lam import atom as _A
        return _A("ERROR"), Dc
    return _d._items(L._list(step_input(x, Dc, fuel)))


def pipe(x, D, a="A", b="B", fuel=None):
    """§14.5: a system form — the composite transition matches component A's output to
    component B's input, the component stores riding as tenant cells of the composite
    store (§14.7). Each component step is the ordinary μ(SYSTEM:x) under its OWN store;
    a component ERROR aborts the composite step with the composite store unchanged
    (§14.3.1, lifted). Backus carries the system forms no further than their existence;
    PIPE is the load-bearing case (process pipelines)."""
    from . import defs as _d
    from .lam import from_lam, atom as _A
    Da, Db = _cell_value(D, a), _cell_value(D, b)
    if Da is None or Db is None:
        return _S(_A("ERROR"), D)
    oa, Da2 = _component_step(x, Da, fuel)
    if from_lam(oa) == "ERROR":
        return _S(_A("ERROR"), D)
    ob, Db2 = _component_step(oa, Db, fuel)
    if from_lam(ob) == "ERROR":
        return _S(_A("ERROR"), D)
    D2 = apply(Store(a), _S(Da2, D))
    return _S(ob, apply(Store(b), _S(Db2, D2)))


def supervise(x, D, child="CHILD", fuel=None):
    """§14.4.4 delegation with reclaim, across a tenant cell: the child store's
    transition runs under fuel; a runaway or erroring child answers ⟨ERROR, composite
    unchanged⟩ — control returns to the parent with the child store intact. Supervision
    is cell nesting plus fuel, no new mechanism."""
    from . import defs as _d
    from .lam import from_lam, atom as _A
    Dc = _cell_value(D, child)
    if Dc is None:
        return _S(_A("ERROR"), D)
    oc, Dc2 = _component_step(x, Dc, fuel)
    if from_lam(oc) == "ERROR":
        return _S(_A("ERROR"), D)
    return _S(oc, apply(Store(child), _S(Dc2, D)))


def child_reset(D, child, x, fuel=None):
    """RESET into a tenant cell: §14.3.2 applied to the CHILD's store — the bootstrap
    when the child has no SYSTEM, the child's OWN transition on x when it does. Note the
    faithful consequence: this cannot repair a child whose SYSTEM diverges (the child
    would just run it again) — pass fuel, or use child_install, the parent's move."""
    Dc = _cell_value(D, child)
    if Dc is None:
        Dc = L.SEQ(L.NIL)
    return apply(Store(child), _S(reset(x, Dc, fuel), D))


def child_install(D, child, name, obj):
    """The parent's prerogative (§14.7): a child's store is a cell of the parent's own,
    so installing a definition in the child — including a NEW SYSTEM for a broken child
    — is an ordinary store BY THE PARENT, no step of the child involved. This, not
    RESET, is how a supervisor repairs a divergent subsystem."""
    Dc = _cell_value(D, child)
    if Dc is None:
        Dc = L.SEQ(L.NIL)
    return apply(Store(child), _S(apply(Store(name), _S(obj, Dc)), D))


def child_retire(D, child):
    """Retire a child system: its cell becomes the empty store (the paper's logical
    deletion, applied to a whole subsystem)."""
    return apply(Store(child), _S(L.SEQ(L.NIL), D))


def run(input_fact, D, validate_obj=None, cell_name="FILE", resolve_obj=None, derive_obj=None, links_obj=None,
        machine=None, mealy_obj=None, fuel=None, index_cell=None, append_cell=None):
    """One AST transition: mu(create_cell:⟨input, D⟩) = ⟨o, D'⟩, with D's OWN definitions in
    scope for the whole step (defs.step — frozen, Backus §14.6). Without a validate it commits
    (V = φ); with validate_of it refuses to commit on an alethic violation; with links_obj the
    representation o carries its HATEOAS links (Thm. hateoas); with machine=(status_cell, sm_obj
    [, entity_role]) the trigger fact advances the noun's machine in this same step (Prop.
    onestep) — and given the entity_role, links_obj is fed the entity's POST-step status, so
    the returned representation offers exactly the next actions (§1: ship, no longer place)."""
    from . import defs
    handler = build_system(validate_obj, cell_name, resolve_obj, derive_obj, links_obj, machine, mealy_obj,
                           index_cell, append_cell)
    with defs.step(D, fuel):
        return _transition(apply(handler, _S(input_fact, D)), D)


# ============================ eq. sys — the whole system as one lambda =========
# SYSTEM : ⟨⟨entity, op⟩, D⟩  →  (rho(↑entity : D)) : ⟨op, D⟩         (the paper's eq. sys)
# The entire running engine is ONE lambda applied to values: D carries every entity's handler
# as a cell (a value); a command names an entity and an operation; the transition fetches that
# entity's handler FROM D (by runtime name), reflects it with rho, and applies it to ⟨op, D⟩.
# An address naming no cell of D fetches # — and #:x reduces to ⊥, so wrong-tenant access is
# not forbidden but impossible (Prop. tenant: isolation = preservation of addressability under ↑).

# DynFetch : ⟨name, D⟩ → contents of the first cell of D named `name` (a runtime
# value), else #. SYSTEM : ⟨⟨entity, op⟩, D⟩ → apply:⟨↑entity:D, ⟨op, D⟩⟩. Both are
# the CANON's (shared/ast.canon, eq. sys verbatim); this module binds them.
from . import canon as _canon
_C = dict(_canon.read("ast.canon"))


def DynFetch():
    """The dynamic fetch expression over ⟨name, D⟩: contents of the first cell of D whose
    name equals the runtime value `name`, else # (the public form of eq. sys's fetch)."""
    return _C["ast:DynFetch"]


SYSTEM = _C["ast:SYSTEM"]


def dispatch(entity, op, D):
    """One eq. sys step: route `op` to the handler that D holds for `entity`, applied to ⟨op, D⟩.
    mu(SYSTEM:⟨⟨entity, op⟩, D⟩), with D's own DEFS in scope (defs.step). An unknown entity
    fetches # and reduces to ⊥ (Prop. tenant)."""
    from . import defs
    with defs.step(D):
        return _transition(apply(SYSTEM, _S(_S(A(entity), op), D)), D)


def _transition(result, D):
    """The AST transition rule (Backus §14.3.1 verbatim): 'If μ(SYSTEM:x) is not a pair,
    the output is an error message and the state remains unchanged.' The transition rules
    are the framework's third element (§14.3), OUTSIDE the applicative subsystem — host
    placement of this check is the faithful placement."""
    from . import defs
    if len(defs._items(L._list(result))) == 2:
        return result
    return _S(A("ERROR"), D)


# ===================== constraints: the violation expressions =====================
"""Constraint families as FFP violation expressions: (rho c) : P = V_c (Def. Violation).

A constraint is *implemented* as an FFP object c; rho (define + the reducer) *reflects*
it back to the ORM layer, where (rho c) : P is the set of population tuples that violate
it. The meta-object 'the constraint' and its FFP implementation are the same entity under
rho — the metamodel describes it, FFP implements it, rho reflects it back. Each family is
authored in Codd theta1 + the Backus base and reduced by the one mu; nothing is host code.
"""
from . import lam as L
from .lam import atom as A, to_lam
from . import canon as T
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
    canonical builder (shared/constraints.canon) applied to the key roles.
        hasDup:⟨t,P⟩ = not null ( Filter(key(1)=key(2) ∧ 1≠2) : (distl:⟨t,P⟩) )
        V_uc         = α(1) ∘ Filter(hasDup) ∘ distr ∘ [id, id]
    """
    from .reduce import apply as _apply
    return _apply(A("constraints:uniqueness"), to_lam(tuple(roles)))


def ring_irreflexive(roles=(1, 2)):
    """Irreflexive ring on a binary fact type: no x relates to itself. V = the
    ⟨x,x⟩ facts. The canonical constraints:ring_irreflexive applied to roles."""
    from .reduce import apply as _apply
    return _apply(A("constraints:ring_irreflexive"), to_lam(tuple(roles)))


def ring_symmetric(roles=(1, 2)):
    """Symmetric ring: x R y implies y R x. V = the ⟨x,y⟩ (x≠y) whose reverse is
    absent from P. The canonical constraints:ring_symmetric applied to roles."""
    from .reduce import apply as _apply
    return _apply(A("constraints:ring_symmetric"), to_lam(tuple(roles)))


def ring_asymmetric(roles=(1, 2)):
    """Asymmetric ring (Halpin §7.3): xRy implies not yRx, reflexive pairs
    violating too. The canonical constraints:ring_asymmetric applied to roles."""
    from .reduce import apply as _apply
    return _apply(A("constraints:ring_asymmetric"), to_lam(tuple(roles)))


def ring_antisymmetric(roles=(1, 2)):
    """Antisymmetric ring (Halpin §7.3): x ≠ y & xRy implies not yRx; reflexive
    pairs allowed. The canonical constraints:ring_antisymmetric applied to roles."""
    from .reduce import apply as _apply
    return _apply(A("constraints:ring_antisymmetric"), to_lam(tuple(roles)))


def ring_intransitive(roles=(1, 2)):
    """Intransitive ring (Halpin §7.3): xRy & yRz implies not xRz. V = P joined
    with its two-step chains. The canonical constraints:ring_intransitive."""
    from .reduce import apply as _apply
    return _apply(A("constraints:ring_intransitive"), to_lam(tuple(roles)))


def ring_acyclic(roles=(1, 2)):
    """Acyclic ring (§7.3): no path via the relation from an object back to
    itself. V = the reflexive pairs of the transitive closure, the closure the
    canonical system:derive_of computes (per Mapping ORM to Datalog). The
    canonical constraints:ring_acyclic."""
    from .reduce import apply as _apply
    return _apply(A("constraints:ring_acyclic"), to_lam(tuple(roles)))


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


def deontic_forbidden(values=None):
    """The forbidden family's validate object (Def. Violation, deontic: flags,
    never blocks). Population form (no values): EVERY row of the forbidden
    fact type violates, so the object is the identity over P (an empty
    population answers no violations). Value form (the old DF_cwa kind): the
    rows carrying any forbidden value violate; setminus leaves exactly those
    rows changed, so sigma selects them."""
    from .reduce import apply as _apply
    from .lam import to_lam as _tl
    if not values:
        return A("id")
    pred = _S(A("COMP"), A("not"), A("eq"),
              _S(A("CONS"),
                 _S(A("COMP"), A("theta:setminus"),
                    _S(A("CONS"), A("id"),
                       _S(A("CONST"), _tl(tuple(values))))),
                 A("id")))
    return _apply(A("theta:Filter"), pred)


def deontic_obligatory_value(values):
    """The obligatory value form's validate object (the old DO_pop kind,
    deontic: flags, never blocks): a row that LACKS every obligated value
    violates. setminus leaves exactly those rows unchanged, so sigma selects
    the complement of deontic_forbidden's set."""
    from .reduce import apply as _apply
    from .lam import to_lam as _tl
    pred = _S(A("COMP"), A("eq"),
              _S(A("CONS"),
                 _S(A("COMP"), A("theta:setminus"),
                    _S(A("CONS"), A("id"),
                       _S(A("CONST"), _tl(tuple(values))))),
                 A("id")))
    return _apply(A("theta:Filter"), pred)


_CC = None


def _canon_c(name):
    global _CC
    if _CC is None:
        from . import canon as _canon
        _CC = dict(_canon.read("constraints.canon"))
    return _CC[name]


def mandatory():
    """Simple mandatory role: every entity plays the role. Input ⟨entities, players⟩;
    V = entities ∖ players (Codd setminus) — the entities that play no fact. The
    canon value (shared/constraints.canon)."""
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
    EXPRESSION (the RMAP view seam) keeps the host composition until system.canon
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


# ===================== system: the pipeline and derive =====================
"""The AREST command pipeline as FFP objects (Def. Command, eq. create), on the kernel.

    create = emit ∘ validate ∘ derive ∘ resolve                         (eq. create)

Each stage is an FFP object reduced by the one mu; nothing is host code. `derive` is the
bounded least fixed point of the immediate-consequence operator (Def. Derive): iterate
F_S from the delta until nothing new is derived, the fixpoint test being set-theoretic —
(F_S:P) ∖ P = φ — since F_S is monotone over a finite fact space (Knaster-Tarski / Lemma
finiteness). Given only the frontier's affected rules (meta.affected_rules), the lfp runs
over the affected fragment, not the whole population (Cor. streaming). `validate` unions
the per-constraint violation sets (rho c):P and flags an alethic offender so the AST step
can refuse to commit (Def. Violation).
"""
from . import lam as L
from .lam import atom as A, PHI
from .defs import define
from . import canon as T

def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)

_COMP, _CONS, _CONST, _COND = A("COMP"), A("CONS"), A("CONST"), A("COND")
_ALPHA, _INSERT, _WHILE = A("ALPHA"), A("INSERT"), A("WHILE")
_ID, _1, _2, _APNDL = A("id"), A(1), A(2), A("apndl")
_EQ, _DISTR, _CAT = A("eq"), A("distr"), A("cat")

# --- default (minimal) stages, as compiled defs ---
define("resolve", _APNDL)                                    # ⟨I,P⟩ → ⟨I, …P⟩  (add the input fact)
define("derive", _ID)                                        # lfp with no rules = id (0 steps)
define("validate", _S(_CONS, _ID, _S(_CONST, PHI)))         # P → ⟨P, φ⟩  (no constraints)
define("emit", _1)                                           # ⟨P,V⟩ → P
define("create", _S(_COMP, A("emit"), A("validate"), A("derive"), A("resolve")))  # eq. create


# --- validate: V = ⋃_c (rho c):P, with the alethic commit guard ---
def validate_of(constraints, alethic=None, scoped=(), scoped_alethic=None):
    """validate_S : ⟨P, D⟩ ↦ ⟨P, V, alethicViolated⟩ (Def. Command / Violation). `constraints`
    consume the target population P (cell-local — composed with the selector); `scoped` consume
    ⟨P, D⟩ whole (cross-cell — they fetch sibling cells from the frozen D). `alethic` /
    `scoped_alethic` are the commit-blocking subsets (default: all of each). The canonical
    builder (shared/system.canon) applied to ⟨local, alethic?, scoped, scoped_alethic?⟩; an
    absent subset is the empty slot, a provided one wraps (deliberately-deontic empties
    stay distinct from absence)."""
    from .reduce import apply as _apply

    def lst(objs):
        out = L.NIL
        for o in reversed(list(objs)):
            out = L.CONS(o)(out)
        return L.SEQ(out)

    def slot(v):
        return _S() if v is None else _S(lst(v))

    rec = _S(lst(constraints), slot(alethic), lst(scoped), slot(scoped_alethic))
    return _apply(A("system:validate_of"), rec)


def validate_modal(pairs, scoped_pairs=()):
    """validate over constraints tagged with modality: pairs = [(obj, modality)] cell-local,
    scoped_pairs likewise for ⟨P,D⟩ consumers. V is the union of ALL violations, but only the
    ALETHIC ones set the block-commit flag (AREST Def. Violation / eq. create). A deontic
    violation is reported in V yet never blocks commit — 'ought to be obeyed but may be
    violated' (the constraint verbalization paper's deontic o)."""
    return validate_of([o for o, _m in pairs],
                       alethic=[o for o, m in pairs if m == "alethic"],
                       scoped=[o for o, _m in scoped_pairs],
                       scoped_alethic=[o for o, m in scoped_pairs if m == "alethic"])


# --- derive = lfp(F_S): the immediate-consequence operator iterated to a fixed point ---
def F_of(rules):
    """One round: F_S(P) = P ∪ ⋃_rules rule(P) (the canonical system:F_of applied
    to the rule sequence; empty rules answer the identity)."""
    from .reduce import apply as _apply
    return _apply(A("system:F_of"), _S(*rules) if rules else _S())


def derive_of(rules):
    """derive_S = lfp(F_S, ·): Backus `while` iterating F_S until (F_S:P) ∖ P = φ
    (the canonical system:derive_of applied to the rule sequence). Pass only the
    frontier's affected rules to keep the lfp bounded to the touched fragment
    (Cor. streaming)."""
    from .reduce import apply as _apply
    return _apply(A("system:derive_of"), _S(*rules) if rules else _S())


# --- role-path -> F_S: a derivation rule is a Datalog rule q(..) <- p1(..), .., pm(..) (ORM ->
# Datalog): a conjunctive query whose body atoms join on shared variables and project to the head
# (the role path). A recursive head (ancestor <- link ; link, ancestor) is resolved by derive_of's
# least fixed point. Each compiled rule is an FFP object P -> its derived head facts. ---
def join_rule(join_role, head_cols):
    """A two-atom SELF-referential role-path rule over one fact type: join the fact type to itself
    on `join_role` (R.join_role = R'.1) and project to `head_cols`. This is the recursive body,
    e.g. ancestor(x,z) <- link(x,y), ancestor(y,z) with head_cols=[1,3], join_role=2 — feed it to
    derive_of for the least fixed point (transitive closure). The canonical
    system:join_rule applied to ⟨join_role, head_cols⟩."""
    from .lam import to_lam
    from .reduce import apply as _apply
    return _apply(A("system:join_rule"), to_lam((join_role, tuple(head_cols))))


def join_rule2(join_role, head_cols):
    """A two-atom role-path rule over two fact types: input ⟨A, B⟩; join A.join_role = B.1 and
    project to head_cols over the combined tuple (e.g. FastCarDriver(x) <- drives(x,y), isFast(y)).
    The canonical system:join_rule2 applied to ⟨join_role, head_cols⟩."""
    from .lam import to_lam
    from .reduce import apply as _apply
    return _apply(A("system:join_rule2"), to_lam((join_role, tuple(head_cols))))


# the storage half of a NORMA */**/+/++ marker: whether the derived facts are materialized (stored)
# vs recomputed on demand. (* and + recompute; ** and ++ store.) The derivation half is the rule
# above, fed to derive_of; the create pipeline runs it as the `derive` stage over the fact's cell.
_MATERIALIZE = {"fully-derived": False, "derived-and-stored": True,
                "semi-derived": False, "partially-derived-and-stored": True}


def materialize(marker):
    """True if the marker means store the derived facts (** / ++), False if compute on demand (* / +)."""
    return _MATERIALIZE.get(marker, False)


# --- resolve with auto-counter minting (Def. Command: mint iff the ref scheme auto-generates) ---
def mint_next(col):
    """P ↦ 1 + the greatest value in column `col` of P (or 1 if empty): the auto-counter's
    next id — successor of a max-fold over the id column (one surrogate per guarded
    step). The canonical system:mint_next applied to the column selector."""
    from .reduce import apply as _apply
    return _apply(A("system:mint_next"), A(col))


def resolve_minting(col):
    """resolve for an auto-generating entity: mint the next id and prepend ⟨id, …I⟩
    to P. The canonical system:resolve_minting applied to the column selector."""
    from .reduce import apply as _apply
    return _apply(A("system:resolve_minting"), A(col))


# --- emit: HATEOAS — the representation carries its own links (Thm. hateoas) ---
# links(e) = nav(e) ∪ transitions(status(e)): the related resources plus the actions available
# from the entity's current state. Both are theta1 selections — nav over P, transitions over a
# state machine value; the representation is self-describing, no link table maintained.
def nav_of(key_pos):
    """nav(e): the facts of P sharing the affected entity's key (role `key_pos` of the head fact).
        α(1) ∘ Filter(key(f) = headKey) ∘ distr ∘ [id, key∘1]
    The canonical builder applied to the key selector (shared/system.canon)."""
    from .reduce import apply as _apply
    return _apply(A("system:nav_of"), A(key_pos))


def transitions_of(sm, status_pos):
    """transitions(status(e)): the state-machine transitions available from the head fact's
    status. `sm` is a value ⟨⟨from, trigger, to⟩…⟩; a transition fires when from = status(head).
        α(1) ∘ Filter(from(t) = status) ∘ distr ∘ [sm̄, status∘1]
    The canonical builder applied to ⟨sm, pos⟩ (shared/system.canon)."""
    from .reduce import apply as _apply
    return _apply(A("system:transitions_of"), _S(sm, A(status_pos)))


def links_of(key_pos, sm=None, status_pos=None):
    """links(e) = nav(e) ∪ transitions(status(e))  (Thm. hateoas). Without a state machine,
    the links are just the navigation."""
    nav = nav_of(key_pos)
    if sm is None:
        return nav
    return _S(_COMP, _CAT, _S(_CONS, nav, transitions_of(sm, status_pos)))  # nav ∪ transitions


# --- S1: membership is application, as a construction (Def. pop / eq. (1)) ---
def typed_fact(ft, values):
    """A fact carrying its type as its first element (the paper's ⟨CONS, s₁…sₙ⟩ shape),
    so the fact 'is resolved by looking up its type' (eq. 1)."""
    from .lam import to_lam
    return to_lam((ft,) + tuple(values))


def typed_population(ft, rows):
    """A population of typed facts: applying it metacomposes down to the TYPE's
    definition, which computes membership — P:g = T iff g ∈ P, one act."""
    from .lam import to_lam
    return to_lam(tuple((ft,) + tuple(r) for r in rows))


def membership_def():
    """The fact type's definition as an operator: metacomposition hands it ⟨f₁, ⟨P, g⟩⟩
    (eq. 1 twice: once on the population, once on the fact), and member:⟨g, P⟩ answers.
    Define this under the fact type's name and the population IS its characteristic
    function, with no new mechanism."""
    g = _S(_COMP, _2, _2)
    P = _S(_COMP, _1, _2)
    return _S(_COMP, T.member, _S(_CONS, g, P))


# --- the book's rule form compiled (Halpin ch.2 ex.4): linear chain join over D ---
def cmp_filter(op, col, lit=None, col2=None):
    """The comparator PREDICATE over a joined row: col `op` lit, or col `op` col2
    (the cross-antecedent form). compile_rule Filter-wraps it after the joins. The
    canonical builders (shared/system.canon)."""
    from .reduce import apply as _apply
    from .lam import to_lam
    if col2 is not None:
        return _apply(A("system:cmp_filter_col"), _S(A(op), A(col), A(col2)))
    return _apply(A("system:cmp_filter_lit"), _S(A(op), A(col), to_lam(lit)))


def _rule_atoms(atom_fts, widths, joins):
    """⟨⟨ft, width, join?⟩…⟩ — the shared atom-spec encoding for the rule builders."""
    from .lam import to_lam
    ws = list(widths) if widths else [2] * len(atom_fts)
    js = list(joins) if joins else [None] * (len(atom_fts) - 1)
    js = [None] + js                                          # first atom never joins
    atoms = L.NIL
    for ftn, w, j in reversed(list(zip(atom_fts, ws, js))):
        spec = to_lam(()) if j is None else _S(to_lam(tuple(tuple(p) for p in j[0])),
                                               to_lam(tuple(j[1])))
        atoms = L.CONS(_S(A(ftn), to_lam(w), spec))(atoms)
    return L.SEQ(atoms)


def _obj_list(objs):
    out = L.NIL
    for o in reversed(list(objs or ())):
        out = L.CONS(o)(out)
    return L.SEQ(out)


def compile_rule(atom_fts, head_positions, widths=None, filters=None, joins=None):
    """A rule's body as one FFP object over D: the populations of the clause fact types,
    each fetched from its own cell, joined linearly by default and by the general Codd
    join where `joins` says so, with the head's variable positions projected. The
    canonical WHILE-over-atoms builder (shared/system.canon) applied to
    ⟨⟨ft, width, join?⟩…, head, filters⟩. Cross-cell by construction:
    store-on-derive's read side."""
    from .reduce import apply as _apply
    from .lam import to_lam
    rec = _S(_rule_atoms(atom_fts, widths, joins), to_lam(tuple(head_positions)),
             _obj_list(filters))
    return _apply(A("system:compile_rule"), rec)


def compile_rule_neg(atom_fts, head_positions, ncols, widths, filters, joins, negs):
    """The positive body wrapped in stratified anti-joins: per negation group
    (neg_atom_fts, neg_key_proj, neg_widths, neg_filters, neg_joins, anti_key),
    a running tuple survives iff its anti_key columns are NOT among the group's
    projected keys (theta:AntiRestrict — Restrict's mirror with the membership
    negated). Full recompute above the closure, exactly like aggregates:
    semi-naive deltas are unsound under negation-as-failure."""
    from .reduce import apply as _apply
    from .lam import to_lam
    obj = compile_rule(atom_fts, list(range(1, ncols + 1)), widths, filters,
                       joins)
    for (nfts, nproj, nwidths, nfilters, njoins, anti_key) in negs:
        neg = compile_rule(nfts, nproj, nwidths, nfilters, njoins)
        # the composition shape is CANONICAL (system:anti_wrap, shared base);
        # this wrapper only marshals the group spec — the wrapper doctrine
        obj = _apply(A("system:anti_wrap"),
                     _S(obj, neg, to_lam((tuple(anti_key),
                                          tuple(range(1, len(nproj) + 1))))))
    head = _apply(A("theta:Project"), to_lam(tuple(head_positions)))
    return _S(A("COMP"), head, obj)


def compile_rule_delta(atom_fts, head_positions, delta_at, widths=None, filters=None,
                       joins=None):
    """The rule body with atom `delta_at` (0-based) reading the round's DELTA instead of
    its cell: an FFP object over ⟨Δ, D⟩ — semi-naive evaluation's inner join
    (Bancilhon–Ramakrishnan 1986). The canonical builder (shared/system.canon) applied to
    ⟨atoms, head, filters, delta_at+1⟩; every non-delta fetch composes with selector 2
    of the pair."""
    from .reduce import apply as _apply
    from .lam import to_lam
    rec = _S(_rule_atoms(atom_fts, widths, joins), to_lam(tuple(head_positions)),
             _obj_list(filters), to_lam(delta_at + 1))
    return _apply(A("system:compile_rule_delta"), rec)


def class_rule(clauses, head_const):
    """A grammar recognizer as one FFP object over D (the parser is the file):
    each clause ⟨field_ft, literal-or-None⟩ selects the Statements whose field
    cell holds the literal (or holds anything); clauses intersect; the head
    pairs survivors with the classification constant. The canonical
    system:class_rule applied to ⟨clauses-with-pred-trees, head⟩; a literal
    becomes the eq-predicate data tree here (the canonical form is the more
    general one: any predicate over the field row)."""
    from .reduce import apply as _apply
    cl = []
    for (ftb, lit) in clauses:
        if lit is None:
            cl.append(_S(A(ftb), _S()))
        else:
            pred = _S(_COMP, _EQ, _S(_CONS, A(2), _S(_CONST, A(lit))))
            cl.append(_S(A(ftb), pred))
    return _apply(A("system:class_rule"),
                  _S(_S(*cl), A(head_const)))


def compile_agg_rule(atom_fts, group_positions, over_position, op,
                     widths=None, filters=None, joins=None):
    """An aggregate rule (Def. derive: 'an aggregate reducing a finite bag to one
    scalar'): joins and filters as compile_rule, then per GROUP (the non-aggregated
    head variables) the fold of `op` over the aggregated column. Stratified above the
    positive closure; the head REPLACES on recompute — an aggregate head is functional
    per group, so union-merge would preserve stale folds (the misfold the old engine
    documented). The canonical builder applied to ⟨atoms, group, over, op, filters⟩."""
    from .reduce import apply as _apply
    from .lam import to_lam
    rec = _S(_rule_atoms(atom_fts, widths, joins), to_lam(tuple(group_positions)),
             to_lam(over_position), A(op), _obj_list(filters))
    return _apply(A("system:compile_agg_rule"), rec)


# FAST twins for classifier rules (stratum 4): keyed by rule cid, fn(D) -> row
# set. Speed as registration under the canonical name — the canonical object in
# D stays the meaning; run_rules consults the twin before generic evaluation.
# Twins rebuild FROM M (classSpec facts freeze with the store), so the thawed
# grammar path gets them too.
rule_twins = {}


def _rowsort(rows):
    """Deterministic row ordering that survives MIXED-TYPE cells: a migrated
    lexical row ('150') and a coerced-arithmetic derivation (150) coexist under
    NATEQ, and bare sorted() has no int-str ordering (the claude rehearsal
    crashed exactly there). Type name then lexical value, per element."""
    return tuple(sorted(rows, key=lambda r: tuple(
        (type(x).__name__, str(x)) for x in r) if isinstance(r, tuple)
        else ((type(r).__name__, str(r)),)))


def rebuild_class_twins(D):
    """Reconstruct class-rule twins from the store's classSpec facts:
    ⟨rid, field_ft, literal-or-empty, head-classification⟩ per clause. The twin
    is the contract from system.class_rule — filter each field cell on column 2
    (or existence), intersect statement ids, pair with the head constant."""
    specs = {}
    for r in _pop_rows(D, "classSpec"):
        if len(r) >= 4:
            specs.setdefault(r[0], ("", []))
            head, clauses = specs[r[0]]
            specs[r[0]] = (r[3], clauses + [(r[1], r[2] or None)])
    for rid, (head, clauses) in specs.items():
        def _twin(Dx, _cl=tuple(clauses), _head=head):
            sids = None
            for (ftb, lit) in _cl:
                rows = _pop_rows(Dx, ftb)
                s = {r[0] for r in rows
                     if r and (lit is None or (len(r) >= 2 and r[1] == lit))}
                sids = s if sids is None else (sids & s)
                if not sids:
                    return set()
            return {(sid, _head) for sid in (sids or ())}
        rule_twins[rid] = _twin
    return len(specs)


def _reconcile_absorbed_heads(D, heads):
    """view == reassembly for DERIVED heads: an ABSORBED head's ** cell is the
    derive cache and its RMAP column is the storage, so after the fixpoint the
    columns become exactly the cell — present rows write their value onto the
    key's table row (fresh keys join the index, hole-padded) and keys whose
    derived row VANISHED hole the column (the sweep's supersession reaches the
    storage). One from_lam/to_lam pass for all heads."""
    from .lam import to_lam, from_lam
    part = rmap_partition(D)
    plan = []
    for ft in heads:
        table = part.get(ft, ft)
        if table == ft:
            continue
        cols = table_columns(part, table)
        col = 2 + cols.index(ft)
        width = 1 + len(cols)
        unary = max((r[2] for r in _pop_rows(D, "role")
                     if len(r) >= 3 and r[1] == ft), default=2) == 1
        plan.append((ft, table, col, width, unary,
                     [tuple(r) for r in _pop_rows(D, ft)]))
    if not plan:
        return D
    from .lam import from_lam as _fl
    cells_l = list(_fl(D))
    idx = {c[1]: i for i, c in enumerate(cells_l)
           if isinstance(c, tuple) and len(c) >= 3 and c[0] == "CELL"}

    def setcell(name, val):
        if name in idx:
            cells_l[idx[name]] = ("CELL", name, val)
        else:
            idx[name] = len(cells_l)
            cells_l.append(("CELL", name, val))

    for (ft, table, col, width, unary, rows) in plan:
        tbl = list(cells_l[idx[table]][2]) if table in idx else []
        keys = {r[0] for r in tbl if r}
        want = {}
        for r in rows:
            if r:
                want[r[0]] = "T" if unary else (r[1] if len(r) >= 2 else "#")
        for k in sorted(keys):
            rc = f"{table}:{k}"
            row = list(cells_l[idx[rc]][2]) if rc in idx                 else [k] + ["#"] * (width - 1)
            while len(row) < width:
                row.append("#")
            v = want.pop(k, "#")
            if row[col - 1] != v:
                row[col - 1] = v
                setcell(rc, tuple(row))
        for k in sorted(want):
            rc = f"{table}:{k}"
            row = list(cells_l[idx[rc]][2]) if rc in idx                 else [k] + ["#"] * (width - 1)
            while len(row) < width:
                row.append("#")
            row[col - 1] = want[k]
            setcell(rc, tuple(row))
            if k not in keys:
                keys.add(k)
                tbl.append((k,))
        setcell(table, tuple(tbl))
    return to_lam(tuple(cells_l))


def run_rules(D, changed=None, stats=None):
    """Cross-cell derivation to the least fixed point, semi-naive (Bancilhon–
    Ramakrishnan 1986): round one evaluates full bodies, BOUNDED by the frontier
    (Cor. streaming — with `changed` given, only rules whose ruleReads intersect fire);
    every later round joins only each head's per-round delta through the stored ~d
    variants (one per atom position, from M's ruleAtom facts), so the join input shrinks
    as the fixpoint nears. Sound and complete because rules are positive and monotone
    and every genuinely new tuple uses at least one new row. Rules without atom facts
    fall back to full evaluation when their reads changed. Rule names resolve through
    D's own DEFS (ρ within the step); Knaster–Tarski gives the lfp and Lemma finiteness
    bounds the rounds. `stats`, when a list, collects per-evaluation records."""
    from . import ast, defs
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam, atom as _A
    reads, atomsof = {}, {}
    for r in _pop_rows(D, "ruleReads"):
        if len(r) >= 2:
            reads.setdefault(r[0], set()).add(r[1])
    # THE INSTANCE MIRROR (proposal B, 2026-07-04): the old engine
    # materialized Resource_is_instance_of_Noun as a reflection cell and the
    # migration dragged 12,539 rows through every derive; pyarest's store IS
    # that knowledge. When any rule reads the mirror, derive it fresh from
    # the role facts (every id playing one of a noun's roles is an instance
    # of that noun) before the closure runs — never migrated, never stale.
    _MIRROR = "Resource_is_instance_of_Noun"
    if any(_MIRROR in rs for rs in reads.values()):
        _mroles = {}
        for r in _pop_rows(D, "role"):
            if len(r) >= 4:
                _mroles.setdefault(r[1], []).append((r[2], r[3]))
        _mnouns = {r[0] for r in _pop_rows(D, "instanceOf")
                   if len(r) >= 2 and r[1] == "ObjectType"}
        _mout = set()
        for _mft, _mrs in _mroles.items():
            _mrows = None
            for (_mpos, _mplayer) in _mrs:
                if _mplayer in _mnouns:
                    if _mrows is None:
                        _mrows = [tuple(x) for x in _pop_rows(D, _mft)]
                    for _mrow in _mrows:
                        if len(_mrow) >= _mpos:
                            _mout.add((_mrow[_mpos - 1], _mplayer))
        if _mout and not _pop_rows(D, _MIRROR):
            # asserted wins: a model carrying its own membership rows keeps
            # them untouched; the mirror serves only the empty cell (the
            # post-migration world, where no reflection row ever lands)
            D = _ap(ast.Store(_MIRROR), _S(to_lam(_rowsort(_mout)), D))
    _FTR = "Fact_Type_has_Role"
    if any(_FTR in rs for rs in reads.values()):
        # the same principle for the role mirror (verdict fourteen's lesson:
        # the arity rule counts it): the role M-facts ARE the knowledge
        _frows = {(r[1], r[0]) for r in _pop_rows(D, "role") if len(r) >= 2}
        if _frows and not _pop_rows(D, _FTR):
            D = _ap(ast.Store(_FTR), _S(to_lam(_rowsort(_frows)), D))
    for r in _pop_rows(D, "ruleAtom"):
        if len(r) >= 3:
            atomsof.setdefault(r[0], []).append((r[1], r[2]))
    aggids = {r[0] for r in _pop_rows(D, "ruleAgg") if r}
    all_rules = [(r[0], r[1]) for r in _pop_rows(D, "ruleDerives")]
    rules = [(rid, h) for (rid, h) in all_rules if rid not in aggids]
    frontier = None if changed is None else set(changed)
    closure_changed = set()
    delta, rnd = None, 0
    while True:
        rnd += 1
        fired, next_delta = False, {}
        for rule_cid, head in rules:
            if delta is None:                                # round one: full bodies
                if frontier is not None and not (reads.get(rule_cid, set()) & frontier):
                    continue
                tw = rule_twins.get(rule_cid)
                if tw is not None:                           # the FAST twin: same rows,
                    new_rows = set(tw(D))                    # no generic evaluation
                else:
                    with defs.step(D):
                        outs = from_lam(_ap(_A(rule_cid), D))
                    if not isinstance(outs, tuple):
                        continue                             # rule not compiled (M-facts only)
                    new_rows = {tuple(r) for r in outs if isinstance(r, tuple)}
                if stats is not None:
                    stats.append({"round": rnd, "rule": rule_cid, "mode": "full"})
            else:
                hits = [(p, ft) for (p, ft) in atomsof.get(rule_cid, ()) if ft in delta]
                if hits:
                    new_rows = set()
                    for (p, ft) in hits:
                        drows = _rowsort(delta[ft])
                        with defs.step(D):
                            o = from_lam(_ap(_A(f"{rule_cid}~d{p}"), _S(to_lam(drows), D)))
                        if isinstance(o, tuple):
                            new_rows |= {tuple(r) for r in o if isinstance(r, tuple)}
                        if stats is not None:
                            stats.append({"round": rnd, "rule": rule_cid, "mode": "delta",
                                          "pos": p, "in": len(drows),
                                          "base": len(_pop_rows(D, ft))})
                elif rule_cid not in atomsof and (reads.get(rule_cid, set()) & set(delta)):
                    tw = rule_twins.get(rule_cid)
                    if tw is not None:
                        new_rows = set(tw(D))
                    else:
                        with defs.step(D):                   # legacy rule: full fallback
                            outs = from_lam(_ap(_A(rule_cid), D))
                        if not isinstance(outs, tuple):
                            continue
                        new_rows = {tuple(r) for r in outs if isinstance(r, tuple)}
                    if stats is not None:
                        stats.append({"round": rnd, "rule": rule_cid, "mode": "full"})
                else:
                    continue
            old = {tuple(r) for r in _pop_rows(D, head)}
            add = new_rows - old
            if add:
                D = _ap(ast.Store(head), _S(to_lam(_rowsort(old | add)), D))
                fired = True
                next_delta.setdefault(head, set()).update(add)
        if not fired:
            break
        delta = next_delta
        closure_changed.update(next_delta)
    # THE UPPER STRATA, iterated to a JOINT fixpoint. Three passes sit above
    # the positive closure, and each can invalidate the others' work through
    # the dependency graph (loads settle, ranks rederives over them, the peak
    # refolds over ranks), so they repeat until a full sweep changes nothing:
    #
    #   agg   — aggregate heads supersede PER GROUP (functional per group;
    #           union would keep stale folds, whole-cell replace clobbered the
    #           corpus's paired zero-supply rows — the count-of-empty lesson);
    #   keyed — heads whose fact type carries a uniqueness over a role prefix
    #           (the old engine's task-955 upsert) re-evaluate over the
    #           settled store and supersede PER KEY; asserted rows whose key
    #           the rules did not produce survive;
    #   sweep — DELETE-AND-REDERIVE (Gupta-Mumick-Subrahmanian 1993, in the
    #           library): for a derivation-OWNED plain head (_OWNED: NORMA's
    #           * and **; + / ++ and unmarked ruled heads keep asserted rows
    #           and stay out of every destructive pass) the stored cell is
    #           materialization of the expressible set (Codd 1970 §1.5), never
    #           ground truth, so it re-evaluates whole and REPLACES — which
    #           both propagates this invocation's supersessions and converges
    #           staleness inherited from earlier stores (frozen caches, replay
    #           history), making derive idempotent. Whole-cell rederivation is
    #           the paper's overestimate-then-rederive at cell granularity,
    #           sound exactly because no row is asserted. Self-supporting
    #           heads (reachable from themselves through derived-head reads)
    #           stay out: their overestimate can rederive itself through the
    #           cycle, and cleaning them needs the delta form of the paper.
    spans_of = {}
    for r in _pop_rows(D, "spans"):
        if len(r) >= 2:
            spans_of.setdefault(r[0], set()).add(r[1])
    keyspans = {}
    for c in _pop_rows(D, "constraint"):
        if len(c) >= 3 and c[1] in ("uniqueness", "spanning_uniqueness"):
            ps = spans_of.get(c[0], set())
            if ps:
                keyspans.setdefault(c[2], set()).update(ps)
    agg_rules = [(rid, head) for (rid, head) in all_rules if rid in aggids]
    agg_heads = {head for (_rid, head) in agg_rules}
    keyed_of = {}
    for (rid, head) in all_rules:
        if rid not in aggids and head in keyspans:
            keyed_of.setdefault(head, []).append(rid)
    plain_of = {}
    for (rid, head) in all_rules:
        if rid not in aggids:
            plain_of.setdefault(head, []).append(rid)
    reach = {h: {ft for rid in rids for ft in reads.get(rid, set())}
             for h, rids in plain_of.items()}
    derived_heads = set(agg_heads) | set(plain_of)
    # MEMBERSHIP comes from THE classification — the same _classify_heads
    # the compile materializes as the passHeads cell — so the running
    # scheduler and the schedule-as-data cannot drift. The rule lists and
    # key positions above stay local: they are the pass bodies' inputs, not
    # the schedule. Self-supporting heads (closures, the 'dred' class) get
    # the paper's RECURSIVE form: a stale cycle rederives itself over a
    # store that still contains it, so whole-cell re-evaluation cannot
    # clean it. Delete the overestimate FIRST (empty the cell), then
    # rederive to the LOCAL least fixpoint from remaining support (GMS93;
    # termination by the same finiteness as the main closure). Rows with
    # only cyclic support die; base-supported rows rebuild.
    classes = _classify_heads(D)
    sweep = classes["sweep"]
    sweep_cyclic = classes["dred"]
    aggwhole = set(classes["aggwhole"])

    def _eval_rules(rids, Dx):
        outs = set()
        for rid in rids:
            tw = rule_twins.get(rid)
            if tw is not None:
                outs |= set(tw(Dx))
                continue
            with defs.step(Dx):
                o = from_lam(_ap(_A(rid), Dx))
            if isinstance(o, tuple):
                outs |= {tuple(r) for r in o if isinstance(r, tuple)}
        return outs

    # Dirty-set filtering keeps the fixpoint's cost proportional to what
    # actually changed: round one evaluates agg and keyed passes whole (their
    # pre-fixpoint status quo) but sweeps only heads whose reads intersect the
    # dirty set — the asserted frontier plus everything the closure and the
    # earlier passes stored this call. A FULL call (changed=None) sweeps every
    # eligible head once, which is where the idempotence guarantee lives; the
    # per-batch delta path pays only for its own ripple. Later rounds filter
    # all three passes the same way, so iteration runs exactly as deep as the
    # dependency chain that moved.
    dirty = None if changed is None else (set(changed) | closure_changed)
    strata_changed = set()
    # the ORDER and the round BOUND come from the canonical constants
    # (system:pass_order / system:pass_bound — the same values
    # scheduler_cells materializes as passOrder/passBound): the joint
    # loop DISPATCHES its native pass bodies by that schedule; an
    # unknown pass name skips (forward compatibility).
    _order = [p for (_i, p) in sorted(
        tuple(r) for r in from_lam(_ap(_A("system:pass_order"), to_lam(())))
        if isinstance(r, tuple) and len(r) >= 2)]
    _brows = from_lam(_ap(_A("system:pass_bound"), to_lam(())))
    _bound = int(_brows[0][0]) if isinstance(_brows, tuple) and _brows \
        and isinstance(_brows[0], tuple) and _brows[0] else 12
    for _outer in range(_bound):
        settled = True
        round_changed = set()

        def _touched(read_set):
            if dirty is None:
                return True
            return bool(read_set & dirty) or bool(read_set & round_changed)
        for _pass in _order:
            if _pass == "agg":
                for (rid, head) in agg_rules:
                    if (_outer or dirty is not None) and not _touched(reads.get(rid, set())):
                        continue
                    with defs.step(D):
                        out = from_lam(_ap(_A(rid), D))
                    if isinstance(out, tuple):
                        agg_rows = {tuple(r) for r in out if isinstance(r, tuple)}
                        before = {tuple(r) for r in _pop_rows(D, head)}
                        if head in aggwhole and dirty is None:
                            # a derivation-OWNED agg head on a FULL derive
                            # replaces whole: the cell is materialization, and
                            # per-group supersession cannot retire a group
                            # whose supply VANISHED (nothing produces its
                            # key). The paired plain rows (the zero-supply
                            # idiom) rejoin fresh.
                            plain = _eval_rules(plain_of.get(head, []), D)
                            merged = agg_rows | plain
                        else:
                            keys = {r[:-1] for r in agg_rows}
                            merged = agg_rows | {r for r in before
                                                 if r[:-1] not in keys}
                        if merged != before:
                            settled = False
                            round_changed.add(head)
                            D = _ap(ast.Store(head), _S(to_lam(_rowsort(merged)), D))
            elif _pass == "keyed":
                for head in classes["keyed"]:
                    if (_outer or dirty is not None) and not _touched(
                            {ft for rid in keyed_of[head] for ft in reads.get(rid, set())}):
                        continue
                    key_pos = sorted(keyspans[head])
                    outs = _eval_rules(keyed_of[head], D)

                    def key(r, _kp=key_pos):
                        return tuple(r[p - 1] for p in _kp if p <= len(r))
                    keys = {key(r) for r in outs}
                    kept = {tuple(r) for r in _pop_rows(D, head)
                            if key(tuple(r)) not in keys}
                    merged = _rowsort(outs | kept)
                    current = _rowsort({tuple(r) for r in _pop_rows(D, head)})
                    if merged != current:
                        settled = False
                        round_changed.add(head)
                        D = _ap(ast.Store(head), _S(to_lam(merged), D))
            elif _pass == "sweep":
                for head in sweep:
                    if not _touched(reach.get(head, set())):
                        continue
                    outs = _eval_rules(plain_of[head], D)
                    current = {tuple(r) for r in _pop_rows(D, head)}
                    if outs != current:
                        settled = False
                        round_changed.add(head)
                        D = _ap(ast.Store(head), _S(to_lam(_rowsort(outs)), D))
            elif _pass == "dred":
                for head in sweep_cyclic:
                    if not _touched(reach.get(head, set())):
                        continue
                    current = {tuple(r) for r in _pop_rows(D, head)}
                    Dx = _ap(ast.Store(head), _S(to_lam(()), D))
                    prev = None
                    outs = set()
                    while outs != prev:
                        prev = outs
                        outs = _eval_rules(plain_of[head], Dx)
                        Dx = _ap(ast.Store(head), _S(to_lam(_rowsort(outs)), Dx))
                    if outs != current:
                        settled = False
                        round_changed.add(head)
                    D = Dx
        if settled:
            break
        dirty = round_changed
        strata_changed.update(round_changed)
    touched = closure_changed | strata_changed
    if touched:
        part_r = rmap_partition(D)
        absorbed = [h for h in sorted(touched)
                    if h in derived_heads and part_r.get(h, h) != h]
        if absorbed:
            D = _reconcile_absorbed_heads(D, absorbed)
    return D


# --- the state machine read off M (whitepaper §1): a machine IS a set of facts ---
# smFrom ⟨t, from⟩ ⋈ smTrigger ⟨t, trigger⟩ ⋈ smTo ⟨t, to⟩, projected to ⟨from, trigger, to⟩ —
# assembling the machine is a theta1 join over M's cells (the canonical defs
# system:sm_join / system:sm_join_named), not a second interpreter. The host
# only fetches and converts: the thin-runner posture, per the Operating Rule
# defs-override-glue-framework.
def sm_triples(D):
    """The machine's ⟨from, trigger, to⟩ triples, joined from M's smFrom/smTrigger/smTo
    cells by the canonical system:sm_join."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    pops = _S(*[_ap(ast.FetchPop(n), D) for n in ("smFrom", "smTrigger", "smTo")])
    return tuple(from_lam(_ap(A("system:sm_join"), pops)))


def sm_triples_named(D):
    """⟨transition, from, trigger, to⟩ — the named form, so per-transition facts
    (guards, Mealy emissions) can key in."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    pops = _S(*[_ap(ast.FetchPop(n), D) for n in ("smFrom", "smTrigger", "smTo")])
    return tuple(from_lam(_ap(A("system:sm_join_named"), pops)))


def machine_for(D, noun):
    """The State Machine Definition governing `noun`: bound directly, or through the
    subtype chain to a bound supertype (a machine tied to a resource SUPERTYPE governs
    its subtypes' instances). None when no machine binds."""
    bound = {r[1]: r[0] for r in _pop_rows(D, "smDef")}
    subs = {r[0]: r[1] for r in _pop_rows(D, "subtype")}
    n, seen = noun, set()
    while n not in bound and n in subs and n not in seen:
        seen.add(n)
        n = subs[n]
    return bound.get(n)


# --- Phase 4 opening: RMAP read off M, driving D's layout (spec §4.4; §Cells) ---
def _pop_rows(D, name):
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    rows = from_lam(_ap(ast.FetchPop(name), D))
    return list(rows) if isinstance(rows, tuple) else []


def rmap_partition(D):
    """M-facts → the cell partition {fact type: table key}, by the RMAP grouping rules. The
    meaning is the canonical system:partition (shared/system.canon), the sub-DEF family
    rmap_top/subject/role2/mand/oneone/side folded to ⟨table, fact type⟩ pairs (Halpin
    ch.10): a spanning UC or none at all gives the fact type its own table (rule 1); a
    single-role UC absorbs it into its role-1 player's top supertype, or the mandatory
    side for a 1:1 (rule 2, §10.3). This host binding applies that def through the reducer,
    so every host reads one definition; the twin is pinned in test_shared_builders."""
    from .lam import atom as A, from_lam
    from .reduce import apply as _apply
    pairs = from_lam(_apply(A("system:partition"), D))
    return {ft: key for (key, ft) in pairs}


def layout_cells(D):
    """Materialize the RMAP layout AS DATA: the rmapColumns cell, rows
    ⟨table, col, ft⟩ for every absorbed fact type, the knowledge
    system:vb_fetch dispatches on (facts all the way down: the partition
    is knowledge about the store, so it rides in the store as a cell).
    Recompile replaces the cell wholesale; a store without it reads as
    all-own-table, which is exactly what a raw compile_model store is."""
    from .lam import to_lam, from_lam
    part = rmap_partition(D)
    rows = []
    for table in sorted({t for ft, t in part.items() if t != ft}):
        for j, ft in enumerate(table_columns(part, table)):
            rows.append((table, 2 + j, ft))
    cells = tuple(c for c in from_lam(D)
                  if not (isinstance(c, tuple) and len(c) >= 2
                          and c[1] == "rmapColumns"))
    return to_lam(cells + (("CELL", "rmapColumns", tuple(rows)),))


# the storage kinds whose cell the DERIVATION owns: no user asserts into a
# * or ** population (NORMA: * recomputes on demand, ** is "kept in sync" —
# both are materialization of the rule's output), so delete-and-rederive is
# sound. Semi-derived (+ / ++) and unmarked ruled heads keep asserted rows
# and never join a destructive pass. Until 2026-07-08 the gates read * only,
# which left every non-keyed ** head silently unmaintained after the 0.9.0
# swap (the tasks board's recommendation columns, the claude app's deontic
# trigger) — resolved engine-side: NORMA's ** is exactly "derive
# materializes into the cell", the same license * carries.
_OWNED = ("fully-derived", "derived-and-stored")


def _classify_heads(D):
    """The joint fixpoint's head classification, ONE computation: pass name →
    sorted heads. 'agg' aggregate-rule heads; 'keyed' key-spanned plain-ruled
    heads (the task-955 upsert; kind-blind, like the pass); among the
    remaining derivation-owned plain heads (_OWNED), 'dred' for the
    self-supporting (empty-first refill, GMS93's recursive form) and 'sweep'
    for the acyclic rest (delete-and-rederive); 'aggwhole' the derivation-
    owned agg heads (the whole-replace license). run_rules builds its strata
    FROM this and scheduler_cells materializes it as the passHeads cell —
    one classification, so the scheduler and the schedule-as-data cannot
    drift. A head may be both 'agg' and 'keyed' (agg rules fold it while
    plain keyed rules upsert it); 'sweep'/'dred' exclude both.

    THE MEANING LIVES IN CANON: system:classify_heads (shared/system.canon,
    the cls_* def family) over the six fetched pops answers the same rows —
    this host function is the certified-equal PERFORMANT OVERRIDE, held
    equal by tests/test_classify_canon.py on every run (the doctrine:
    functionality with a performant override must be defined in the shared
    lambda base)."""
    aggids = {r[0] for r in _pop_rows(D, "ruleAgg") if r}
    all_rules = [(r[0], r[1]) for r in _pop_rows(D, "ruleDerives")
                 if len(r) >= 2]
    agg_heads = {h for (rid, h) in all_rules if rid in aggids}
    plain_of = {}
    for (rid, h) in all_rules:
        if rid not in aggids:
            plain_of.setdefault(h, []).append(rid)
    kindmap = {r[0]: r[1] for r in _pop_rows(D, "derivation") if len(r) >= 2}
    spans_of = {}
    for r in _pop_rows(D, "spans"):
        if len(r) >= 2:
            spans_of.setdefault(r[0], set()).add(r[1])
    keyspanned = set()
    for c in _pop_rows(D, "constraint"):
        if len(c) >= 3 and c[1] in ("uniqueness", "spanning_uniqueness") \
                and spans_of.get(c[0]):
            keyspanned.add(c[2])
    reads = {}
    for r in _pop_rows(D, "ruleReads"):
        if len(r) >= 2:
            reads.setdefault(r[0], set()).add(r[1])
    reach = {h: {ft for rid in rids for ft in reads.get(rid, set())}
             for h, rids in plain_of.items()}
    derived_heads = set(agg_heads) | set(plain_of)

    def _self_supporting(h):
        seen, stack = set(), [x for x in reach.get(h, ()) if x in derived_heads]
        while stack:
            x = stack.pop()
            if x == h:
                return True
            if x in seen:
                continue
            seen.add(x)
            stack.extend(y for y in reach.get(x, ()) if y in derived_heads)
        return False

    owned = [h for h in plain_of
             if kindmap.get(h) in _OWNED
             and h not in agg_heads and h not in keyspanned]
    return {"agg": sorted(agg_heads),
            "keyed": sorted(h for h in plain_of if h in keyspanned),
            "sweep": sorted(h for h in owned if not _self_supporting(h)),
            "dred": sorted(h for h in owned if _self_supporting(h)),
            # the agg pass's whole-replace-vs-per-group decision is a KIND
            # question pass membership alone cannot carry: a derivation-
            # owned agg head on a FULL derive replaces whole (a vanished
            # group dies); any other agg head supersedes per group. The
            # fifth label rides the cell so readers never need kindmap.
            "aggwhole": sorted(h for h in agg_heads
                               if kindmap.get(h) in _OWNED)}


def scheduler_cells(D):
    """Materialize the SCHEDULE as data (the pipeline-as-data endgame's first
    face): the passHeads cell, rows ⟨pass, head⟩ from _classify_heads — the
    same classification run_rules builds its strata from, computed once at
    compile beside rmapColumns. The pass BODIES stay native (the certified
    fast lane); the MEMBERSHIP becomes store knowledge any host reads instead
    of recomputing. Recompile replaces the cell wholesale; a store without it
    classifies at run time, which is what run_rules does anyway."""
    from .lam import to_lam, from_lam, atom as _A
    from .reduce import apply as _ap
    classes = _classify_heads(D)
    rows = [(p, h) for p in ("agg", "keyed", "sweep", "dred", "aggwhole")
            for h in classes[p]]
    # the ORDER and the round BOUND are constants of doctrine (canonical
    # defs system:pass_order / system:pass_bound) — evaluated here and
    # materialized beside the membership, so a reader holds the whole
    # schedule: which passes, whose heads, in what order, bounded how.
    order = from_lam(_ap(_A("system:pass_order"), to_lam(())))
    bound = from_lam(_ap(_A("system:pass_bound"), to_lam(())))
    cells = tuple(c for c in from_lam(D)
                  if not (isinstance(c, tuple) and len(c) >= 2
                          and c[1] in ("passHeads", "passOrder",
                                       "passBound")))
    return to_lam(cells + (("CELL", "passHeads", tuple(rows)),
                           ("CELL", "passOrder", tuple(order)),
                           ("CELL", "passBound", tuple(bound))))


def generator_cells(D):
    """The generator family's first member (punchlist entry 8): dsl:<Noun>
    cells, the per-noun model summary the old engine persists (noun, object
    type, reading texts, verbalized constraints as kind-text pairs, machine
    transitions). Computed from M at compile time beside the layout cells;
    recompile replaces the family wholesale."""
    from .lam import to_lam, from_lam
    kinds = {}
    for r in _pop_rows(D, "instanceOf"):
        if len(r) >= 2 and r[1] in ("ObjectType", "ValueType"):
            kinds[r[0]] = "entity" if r[1] == "ObjectType" else "value"
    roles = {}
    for r in _pop_rows(D, "role"):
        if len(r) >= 4:
            roles.setdefault(r[1], []).append((r[2], r[3]))
    readings = {}
    for f in _pop_rows(D, "factType"):
        if len(f) >= 2 and f[0] in roles:
            players = [p for (_i, p) in sorted(roles[f[0]])]
            try:
                readings[f[0]] = str(f[1]).format(*players)
            except (IndexError, KeyError):
                readings[f[0]] = str(f[1])
    cons = []
    for c in _pop_rows(D, "constraint"):
        if len(c) < 3:
            continue
        ft, players = c[2], [p for (_i, p) in sorted(roles.get(c[2], []))]
        if c[1] == "uniqueness" and len(players) >= 2:
            cons.append((ft, players, ("UC", "Each %s has at most one %s."
                                       % (players[0], players[1]))))
        elif c[1] == "mandatory" and len(players) >= 2:
            cons.append((ft, players, ("MC", "Each %s has some %s."
                                       % (players[0], players[1]))))
        elif str(c[1]).startswith("deontic"):
            cons.append((ft, players, ("UC", str(c[0]) + ".")))
    # the machine triples per governed noun; a triple does not carry its
    # machine id, so with several machines every governed noun sees the
    # union (the single-machine case, every app in the fleet today, is
    # exact; the defined-in link refines it when a multi-machine app lands)
    sms = {}
    triples = [(trig, frm, to) for (frm, trig, to) in sm_triples(D)]
    for r in _pop_rows(D, "smDef"):
        if len(r) >= 2:
            sms.setdefault(r[1], []).extend(triples)
    cells = {}
    for noun, kind in sorted(kinds.items()):
        my_fts = [ft for ft, ps in roles.items()
                  if any(p == noun for (_i, p) in ps)]
        my_readings = tuple(sorted(readings[ft] for ft in my_fts
                                   if ft in readings))
        my_cons = tuple(pair for (ft, players, pair) in cons
                        if noun in players)
        my_trans = tuple(sorted(set(sms.get(noun, ()))))
        cells["dsl:" + noun] = ((noun, kind, my_readings, my_cons,
                                 my_trans),)
    # the OPT-IN family (docs/07-generators.md restored 2026-07-08; the
    # runtime-parity list owl xsd edm html dtd wsdl xforms plix nav,
    # NORMA's XML/OIALto* transforms the on-disk oracle at
    # Repos/NORMA/XML). App uses Generator '<name>' instance facts
    # activate targets; a generator not opted in produces nothing.
    # Per entity noun THE CANON classifies once (system:ev_cols — the
    # generator never re-derives the layout; system:sqlname names) and
    # each opted format transduces the same classification. Column
    # kinds: unary -> boolean, ref -> the target noun, value -> string
    # until datatype facts land. Identity is the id attribute/key/part
    # everywhere (eq. (sys)); transitions come from the dsl member's
    # machine triples (Theorem 4a); plural slugs prefer Noun has
    # Plural instance facts over sqlname+s (docs/07).
    opted = {r[1] for r in _pop_rows(D, "App_uses_Generator")
             if len(r) >= 2}
    active = opted.intersection(("xsd", "owl", "edm", "html", "dtd",
                                 "wsdl", "xforms", "plix", "nav",
                                 "solidity"))
    if active:
        import json as _json
        from .reduce import apply as _xap
        from .lam import atom as _XA
        import pyarest.lam as _XL

        def _xpair(noun_name):
            return _XL.SEQ(_XL.CONS(_XA(noun_name))(_XL.CONS(D)(_XL.NIL)))

        def _xesc(s):
            return (str(s).replace("&", "&amp;").replace("<", "&lt;")
                    .replace(">", "&gt;").replace('"', "&quot;"))

        def _sqlname(s):
            return str(from_lam(_xap(_XA("system:sqlname"),
                                     _XA(str(s)))))

        def _xname(s):
            return "".join(str(s).split())

        def _pascal(s):
            return "".join(w.capitalize() for w in str(s).split("_"))

        plurals = {r[0]: r[1] for r in _pop_rows(D, "Noun_has_Plural")
                   if len(r) >= 2}

        def _plural(noun_name):
            return plurals.get(noun_name, _sqlname(noun_name) + "s")

        for noun, kind in sorted(kinds.items()):
            if kind != "entity":
                continue
            classified = from_lam(_xap(_XA("system:ev_cols"),
                                       _xpair(noun)))
            if not isinstance(classified, tuple):
                continue
            cols = [(str(c[0]), str(c[1]), c[2], str(c[3]))
                    for c in classified
                    if isinstance(c, tuple) and len(c) >= 4]
            trans = cells["dsl:" + noun][0][4]
            events = sorted({t[0] for t in trans if len(t) >= 3})
            x = _xname(noun)
            s_noun = _sqlname(noun)
            p_noun = _plural(noun)

            if "xsd" in active:
                lines = ['<xs:complexType name="%s">' % _xesc(x)]
                lines.append("  <xs:sequence>")
                for _ft, ckind, _other, cname in cols:
                    ctype = ("xs:boolean" if ckind == "unary"
                             else "xs:string")
                    lines.append(
                        '    <xs:element name="%s" type="%s"'
                        ' minOccurs="0"/>' % (_xesc(cname), ctype))
                lines.append("  </xs:sequence>")
                lines.append('  <xs:attribute name="id" type="xs:string"'
                             ' use="required"/>')
                lines.append("</xs:complexType>")
                cells["xsd:" + noun] = (("\n".join(lines),),)

            if "owl" in active:
                XSDNS = "http://www.w3.org/2001/XMLSchema#"
                lines = ['<owl:Class rdf:about="#%s"/>' % _xesc(x)]
                for _ft, ckind, other, cname in cols:
                    is_ref = ckind == "ref" and other is not None
                    tag = ("owl:ObjectProperty" if is_ref
                           else "owl:DatatypeProperty")
                    rng = ("#" + _xname(other) if is_ref
                           else XSDNS + ("boolean" if ckind == "unary"
                                         else "string"))
                    lines.append('<%s rdf:about="#%s.%s">'
                                 % (tag, _xesc(x), _xesc(cname)))
                    lines.append('  <rdfs:domain rdf:resource="#%s"/>'
                                 % _xesc(x))
                    lines.append('  <rdfs:range rdf:resource="%s"/>'
                                 % _xesc(rng))
                    lines.append("</%s>" % tag)
                cells["owl:" + noun] = (("\n".join(lines),),)

            if "edm" in active:
                lines = ['<EntityType Name="%s">' % _xesc(x)]
                lines.append("  <Key>")
                lines.append('    <PropertyRef Name="id"/>')
                lines.append("  </Key>")
                lines.append('  <Property Name="id" Type="Edm.String"'
                             ' Nullable="false"/>')
                for _ft, ckind, other, cname in cols:
                    if ckind == "ref" and other is not None:
                        lines.append(
                            '  <NavigationProperty Name="%s" Type="%s"/>'
                            % (_xesc(cname), _xesc(_xname(other))))
                    else:
                        etype = ("Edm.Boolean" if ckind == "unary"
                                 else "Edm.String")
                        lines.append(
                            '  <Property Name="%s" Type="%s"'
                            ' Nullable="true"/>' % (_xesc(cname), etype))
                lines.append("</EntityType>")
                cells["edm:" + noun] = (("\n".join(lines),),)

            if "html" in active:
                lines = ['<form data-noun="%s" method="post"'
                         ' action="/%s">' % (_xesc(noun), _xesc(p_noun))]
                lines.append('  <label>id <input name="id" type="text"'
                             ' required/></label>')
                for _ft, ckind, other, cname in cols:
                    itype = "checkbox" if ckind == "unary" else "text"
                    ref = ('' if not (ckind == "ref" and other is not None)
                           else ' data-ref="%s"' % _xesc(other))
                    lines.append(
                        '  <label>%s <input name="%s" type="%s"%s/>'
                        '</label>' % (_xesc(cname), _xesc(cname),
                                      itype, ref))
                lines.append('  <button type="submit">Create %s</button>'
                             % _xesc(noun))
                lines.append("</form>")
                cells["html:" + noun] = (("\n".join(lines),),)

            if "dtd" in active:
                model = ("(%s)" % ", ".join(c[3] + "?" for c in cols)
                         if cols else "EMPTY")
                lines = ["<!ELEMENT %s %s>" % (s_noun, model),
                         "<!ATTLIST %s id CDATA #REQUIRED>" % s_noun]
                for _ft, _ckind, _other, cname in cols:
                    lines.append("<!ELEMENT %s (#PCDATA)>" % cname)
                cells["dtd:" + noun] = (("\n".join(lines),),)

            if "wsdl" in active:
                lines = ['<wsdl:message name="%sGetRequest">' % x,
                         '  <wsdl:part name="id" type="xsd:string"/>',
                         "</wsdl:message>",
                         '<wsdl:message name="%sCreateRequest">' % x,
                         '  <wsdl:part name="body" element="tns:%s"/>' % x,
                         "</wsdl:message>",
                         '<wsdl:message name="%sResponse">' % x,
                         '  <wsdl:part name="body" element="tns:%s"/>' % x,
                         "</wsdl:message>"]
                ops = [("get" + x, "GetRequest"),
                       ("create" + x, "CreateRequest")]
                ops += [(_xname(e) + x, "GetRequest") for e in events]
                lines.append('<wsdl:portType name="%sPort">' % x)
                for opname, req in ops:
                    lines.append('  <wsdl:operation name="%s">'
                                 % _xesc(opname))
                    lines.append('    <wsdl:input message="tns:%s%s"/>'
                                 % (x, req))
                    lines.append('    <wsdl:output'
                                 ' message="tns:%sResponse"/>' % x)
                    lines.append("  </wsdl:operation>")
                lines.append("</wsdl:portType>")
                cells["wsdl:" + noun] = (("\n".join(lines),),)

            if "xforms" in active:
                lines = ['<xf:model id="%s">' % x,
                         "  <xf:instance>",
                         '    <%s id="">' % s_noun]
                for _ft, _ckind, _other, cname in cols:
                    lines.append("      <%s/>" % cname)
                lines.append("    </%s>" % s_noun)
                lines.append("  </xf:instance>")
                lines.append('  <xf:bind nodeset="@id"'
                             ' required="true()"/>')
                for _ft, ckind, _other, cname in cols:
                    btype = ("xf:boolean" if ckind == "unary"
                             else "xf:string")
                    lines.append('  <xf:bind nodeset="%s" type="%s"/>'
                                 % (cname, btype))
                lines.append("</xf:model>")
                for _ft, _ckind, _other, cname in cols:
                    lines.append('<xf:input ref="%s"><xf:label>%s'
                                 "</xf:label></xf:input>"
                                 % (cname, _xesc(cname)))
                lines.append('<xf:submit submission="create-%s">'
                             "<xf:label>Create %s</xf:label></xf:submit>"
                             % (s_noun, _xesc(noun)))
                cells["xforms:" + noun] = (("\n".join(lines),),)

            if "plix" in active:
                lines = ['<plx:class name="%s" visibility="public">' % x]
                props = [("Id", ".string")]
                for _ft, ckind, other, cname in cols:
                    if ckind == "ref" and other is not None:
                        props.append((_pascal(cname), _xname(other)))
                    else:
                        props.append((_pascal(cname),
                                      ".boolean" if ckind == "unary"
                                      else ".string"))
                for pname, ptype in props:
                    lines.append('  <plx:property name="%s"'
                                 ' visibility="public">' % _xesc(pname))
                    lines.append('    <plx:returns dataTypeName="%s"/>'
                                 % _xesc(ptype))
                    lines.append("  </plx:property>")
                lines.append("</plx:class>")
                cells["plix:" + noun] = (("\n".join(lines),),)

            if "nav" in active:
                navs = [{"relation": ft, "target": str(other),
                         "href": "/%s/{id}/%s"
                                 % (p_noun, _plural(str(other)))}
                        for ft, ckind, other, _cname in cols
                        if ckind == "ref" and other is not None]
                trs = [{"event": t[0], "from": t[1], "to": t[2],
                        "href": "/%s/{id}/%s" % (p_noun, t[0])}
                       for t in trans if len(t) >= 3]
                doc = {"noun": noun, "self": "/%s/{id}" % p_noun,
                       "navigation": navs, "transitions": trs}
                cells["nav:" + noun] = (
                    (_json.dumps(doc, sort_keys=True),),)

            if "solidity" in active:
                # the oracle: generators/solidity.rs (d3104058~1). The
                # machine's bytes32 status REPLACES the status value
                # column (0.9: status(e) = RMAP column, one meaning)
                def _camel(s):
                    w = _pascal(s)
                    return w[:1].lower() + w[1:] if w else w

                has_sm = bool(trans)
                scols = [(ft, ckind, other, cname)
                         for ft, ckind, other, cname in cols
                         if not (has_sm and cname == "status")]
                lines = ["// SPDX-License-Identifier: MIT",
                         "// Generated from FORML2 readings by AREST",
                         "pragma solidity ^0.8.20;",
                         "",
                         "contract %s {" % x,
                         "    struct Data {",
                         "        string id;"]
                for _ft, ckind, _other, cname in scols:
                    stype = "bool" if ckind == "unary" else "string"
                    lines.append("        %s %s;" % (stype, _camel(cname)))
                if has_sm:
                    lines.append("        bytes32 status;"
                                 "  // SM current state")
                lines.append("    }")
                lines.append("")
                lines.append("    mapping(string => Data) public"
                             " records;")
                lines.append("")
                for ft, ckind, _other, cname in cols:
                    stype = "bool" if ckind == "unary" else "string"
                    lines.append("    event %s(string indexed id,"
                                 " %s %s);" % (_pascal(ft), stype,
                                               _camel(cname)))
                if has_sm:
                    lines.append("")
                    lines.append("    modifier onlyInStatus(string"
                                 " memory id, bytes32 expected) {")
                    lines.append("        require(records[id].status =="
                                 ' expected, "SM: wrong state");')
                    lines.append("        _;")
                    lines.append("    }")
                params = ["string memory id"]
                for _ft, ckind, _other, cname in scols:
                    stype = "bool" if ckind == "unary" else "string"
                    params.append("%s memory %s" % (stype, _camel(cname))
                                  if stype == "string"
                                  else "%s %s" % (stype, _camel(cname)))
                lines.append("")
                lines.append("    function create(%s) external {"
                             % ", ".join(params))
                lines.append("        require(bytes(records[id].id)"
                             '.length == 0, "UC: %s already exists");'
                             % x)
                mc_fields = set()
                for ckind_, text in (cells["dsl:" + noun][0][3] or ()):
                    if ckind_ == "MC" and " some " in str(text):
                        tail = str(text).split(" some ", 1)[1]
                        tail = tail.rstrip(".").split(",")[0].strip()
                        if tail:
                            mc_fields.add(_camel(_sqlname(tail)))
                for _ft, ckind, _other, cname in scols:
                    f = _camel(cname)
                    if f in mc_fields and ckind != "unary":
                        lines.append("        require(bytes(%s).length"
                                     ' > 0, "MC: %s required");'
                                     % (f, cname))
                lines.append("        records[id].id = id;")
                for ft, ckind, _other, cname in scols:
                    f = _camel(cname)
                    lines.append("        records[id].%s = %s;" % (f, f))
                    lines.append("        emit %s(id, %s);"
                                 % (_pascal(ft), f))
                initial = sorted({t[1] for t in trans if len(t) >= 3
                                  and t[1]})
                if has_sm and initial:
                    lines.append("        records[id].status ="
                                 ' keccak256(bytes("%s"));' % initial[0])
                lines.append("    }")
                for trig, frm, to in sorted(set(
                        t for t in trans if len(t) >= 3)):
                    lines.append("")
                    lines.append("    function %s(string memory id)"
                                 " external onlyInStatus(id,"
                                 ' keccak256(bytes("%s"))) {'
                                 % (_camel(_sqlname(trig)), frm))
                    lines.append("        records[id].status ="
                                 ' keccak256(bytes("%s"));' % to)
                    lines.append("    }")
                lines.append("}")
                cells["solidity:" + noun] = (("\n".join(lines),),)
    _GEN = ("dsl:", "xsd:", "owl:", "edm:", "html:", "dtd:", "wsdl:",
            "xforms:", "plix:", "nav:", "solidity:")
    keep = tuple(c for c in from_lam(D)
                 if not (isinstance(c, tuple) and len(c) >= 2
                         and str(c[1]).startswith(_GEN)))
    fresh = tuple(("CELL", name, rows) for name, rows in sorted(cells.items()))
    return to_lam(keep + fresh)


def absorb_rows(D, table_key, partition):
    """The 3NF row population of one RMAP table: the θ₁ natural join on the key (role 1)
    of the fact types absorbed into `table_key` (spec §4.4: functional roles on the same
    object type give one cell keyed on its id). Entities missing a functional fact drop
    from the joined rows; the optional-column (outer join) refinement is a later step."""
    from . import canon as T
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam
    import pyarest.lam as L
    fts = [ft for ft, key in partition.items() if key == table_key and ft != table_key]
    if not fts:
        return []
    acc = to_lam(tuple(tuple(r) for r in _pop_rows(D, fts[0])))
    for ft in fts[1:]:
        nxt = to_lam(tuple(tuple(r) for r in _pop_rows(D, ft)))
        acc = _ap(T.NatJoin(1), L.SEQ(L.CONS(acc)(L.CONS(nxt)(L.NIL))))
    return list(from_lam(acc))


# The cell-naming boundary op, as the reference TS engine computes it in the worker
# (12-physical-mapping.md: cellKey('Order','org-1') gives 'Order:org-1'). Strings are
# outside the algebra, so joining a name is a registered value op (spec D5).
def _cellkey_impl(mu):
    from . import defs as _d
    import pyarest.lam as L

    def g(o):
        it = _d._items(L._list(o))
        if len(it) != 2:
            return L.BOT
        a, b = _d._aval(it[0]), _d._aval(it[1])
        if a is None or b is None or isinstance(a, tuple) or isinstance(b, tuple):
            return L.BOT
        return L.atom(f"{a}:{b}")
    return g


def _register_cellkey():
    from .defs import register
    register("cellkey", _cellkey_impl)


_register_cellkey()


# The skolem boundary op (task-970's value-invention leaf, mapped to 0.9.0):
# the semi-oblivious chase's labelled null — an existential head's fresh id
# as a PURE function of its frontier. apply(skolem, ⟨v1..vn⟩) answers
# 've_' + fnv1a64_hex(v1 '|' .. '|' vn). ID MINTING is a boundary act (the
# slug precedent, same D5 slot); determinism is the idempotence crux: the
# same frontier answers the same id, so the owned sweep DEDUPS a
# re-derivation instead of duplicating it — eager delete-and-rederive IS
# the chase step. An empty or non-sequence input answers ⊥ (a skolem with
# no distinguishing frontier is a modeling error, not a global singleton).
def _skolem_impl(mu):
    from . import defs as _d
    import pyarest.lam as L

    def g(o):
        it = _d._items(L._list(o))
        if not it:
            return L.BOT
        vals = []
        for x in it:
            v = _d._aval(x)
            # str and int atoms only (the cross-host pin's domain): a float
            # frontier would format differently per host — refuse it
            if not isinstance(v, (str, int)) or isinstance(v, bool):
                return L.BOT
            vals.append(str(v))
        h = 14695981039346656037
        for byte in "|".join(vals).encode("utf-8"):
            h = ((h ^ byte) * 1099511628211) % (1 << 64)
        return L.atom("ve_" + format(h, "016x"))
    return g


def _register_skolem():
    from .defs import register
    register("skolem", _skolem_impl)


_register_skolem()


# The html ESCAPE transducer — the render's ONE legitimate boundary piece
# (the doctrine correction, 2026-07-08: meaning in canon, boundary for
# TRANSDUCTION only). Byte-level entity substitution, the lex family:
# & < > " to their entities; ints stringify; sequences bottom.
def _escape_html_impl(mu):
    from . import defs as _d
    import pyarest.lam as L

    def g(o):
        v = _d._aval(o)
        if v is None or isinstance(v, tuple) or isinstance(v, bool):
            return L.BOT
        s = (str(v).replace("&", "&amp;").replace("<", "&lt;")
             .replace(">", "&gt;").replace('"', "&quot;"))
        return L.atom(s)
    return g


def _register_escape_html():
    from .defs import register
    register("escape_html", _escape_html_impl)


_register_escape_html()


# The prefix-strip base op — generic string algebra beside implode and
# slug (spec D5): ⟨prefix, s⟩ answers s with a leading prefix removed,
# or s unchanged. No policy — the CHOICE of what to strip is canon's
# (system:sqlcol_base strips the noun off a unary fact type's name).
def _strip_prefix_impl(mu):
    from . import defs as _d
    import pyarest.lam as L

    def g(o):
        it = _d._items(L._list(o))
        if len(it) != 2:
            return L.BOT
        vals = []
        for x in it:
            v = _d._aval(x)
            if not isinstance(v, (str, int)) or isinstance(v, bool):
                return L.BOT
            vals.append(str(v))
        pre, s = vals
        return L.atom(s[len(pre):] if s.startswith(pre) else s)
    return g


# The JSON view emitter — the react/Worker render target (the binding
# doctrine: a react component consumes the element TREE + the apply
# endpoint and nothing else, so the "render" is the tree itself as
# JSON — a pure transducer beside render:html, no widget knowledge).
def _render_json_impl(mu):
    from . import defs as _d
    import pyarest.lam as L
    import json

    def _plain(o):
        v = _d._aval(o)
        if v is not None and not isinstance(v, tuple):
            return v
        it = _d._items(L._list(o))
        return [_plain(x) for x in it]

    def g(o):
        try:
            return L.atom(json.dumps(_plain(o), ensure_ascii=False,
                                     separators=(",", ":")))
        except Exception:
            return L.BOT
    return g


def _register_render_json():
    from .defs import register
    register("render:json", _render_json_impl)


_register_render_json()


def _register_strip_prefix():
    from .defs import register
    register("strip_prefix", _strip_prefix_impl)


_register_strip_prefix()


# The reference RENDER function (AREST.tex §Platform binding, verbatim:
# "Binding a user interface is then registering a render function, so a
# fact renders itself"). MEANING IN CANON: system:render_html is the
# definition of record (tree structure + implode joins over escape_html);
# this host function is the certified-equal PERFORMANCE OVERRIDE, held
# equal by the twin test — the _classify_heads discipline. Toolkit
# renderers (slint, gtk, web-components…) register beside it the same
# way over the SAME trees: the iFactr pattern.
def _render_html_impl(mu):
    from . import defs as _d
    import pyarest.lam as L

    def esc(s):
        return (str(s).replace("&", "&amp;").replace("<", "&lt;")
                .replace(">", "&gt;").replace('"', "&quot;"))

    def g(o):
        it = _d._items(L._list(o))
        if len(it) != 2:
            return L.BOT
        kind = _d._aval(it[0])
        rows = _d._items(L._list(it[1]))
        if kind == "menu":
            parts = []
            for r in rows:
                rr = _d._items(L._list(r))
                if len(rr) != 3:
                    return L.BOT
                ev, to = _d._aval(rr[1]), _d._aval(rr[2])
                parts.append(f'<button name="{esc(ev)}" value="{esc(to)}">'
                             f'{esc(ev)}</button>')
            return L.atom('<nav class="menu">' + "".join(parts) + "</nav>")
        if kind == "detail":
            parts = []
            for r in rows:
                rr = _d._items(L._list(r))
                if len(rr) != 3:
                    return L.BOT
                reading = _d._aval(rr[1])
                vals = [_d._aval(x) for x in _d._items(L._list(rr[2]))]
                parts.append(f"<dt>{esc(reading)}</dt>"
                             f"<dd>{esc(' '.join(str(v) for v in vals))}</dd>")
            return L.atom('<dl class="detail">' + "".join(parts) + "</dl>")
        if kind == "list":
            parts = []
            for r in rows:
                rr = _d._items(L._list(r))
                if len(rr) != 3:
                    return L.BOT
                rid, cap = _d._aval(rr[1]), _d._aval(rr[2])
                parts.append(f'<li data-id="{esc(rid)}">{esc(cap)}</li>')
            return L.atom('<ul class="list">' + "".join(parts) + "</ul>")
        return L.BOT
    return g


def _register_render_html():
    from .defs import register
    register("render:html", _render_html_impl)


_register_render_html()


# The tokenizer boundary (the keystone's transducer set, same D5 slot): three
# value ops carry text into the object world — lex (text → per-word records),
# implode (⟨sep, words⟩ → one atom; templates are strings in factType rows),
# slug (text → id atom; ID MINTING is a boundary act, names are data). All
# sequence algebra above them (the mixfix scan, type spans, Stage-1's
# vocabulary matcher) is canonical territory.
def _lex_impl(mu):
    import re
    from . import defs as _d
    from .lam import to_lam
    import pyarest.lam as L

    def g(o):
        t = _d._aval(o)
        if t is None or isinstance(t, tuple):
            return L.BOT
        text = str(t)
        spans = [m.span() for m in re.finditer(r"'[^']*'", text)]
        rows = []
        for m in re.finditer(r"\S+", text):
            tok, s, e = m.group(0), m.start(), m.end()
            k = next((i + 1 for i, (a, b) in enumerate(spans)
                      if s < b and a < e), 0)
            qtext = ""
            if k:
                a, b = spans[k - 1]
                qtext = text[max(s, a + 1):min(e, b - 1)]
            nopunct = tok.strip(".;:,")
            base = nopunct.rstrip("0123456789")
            rows.append((tok, nopunct, base, nopunct[len(base):], tok.lower(),
                         qtext, "T" if base and base[0].isupper() else "F",
                         tok.partition("-")[2], "T" if k else "F", k))
        return to_lam(tuple(rows))
    return g


def _implode_impl(mu):
    from . import defs as _d
    import pyarest.lam as L

    def g(o):
        it = _d._items(L._list(o))
        if len(it) != 2:
            return L.BOT
        sep = _d._aval(it[0])
        if sep is None or isinstance(sep, tuple):
            return L.BOT
        parts = []
        for w in _d._items(L._list(it[1])):
            v = _d._aval(w)
            if v is None or isinstance(v, tuple):
                return L.BOT
            parts.append(str(v))
        return L.atom(str(sep).join(parts))
    return g


def _slug_impl(mu):
    import re
    from . import defs as _d
    import pyarest.lam as L

    def g(o):
        t = _d._aval(o)
        if t is None or isinstance(t, tuple):
            return L.BOT
        return L.atom(re.sub(r"[^0-9A-Za-z]+", "_", str(t)).strip("_"))
    return g


_S1_QUOTED_SPAN = None
_S1_QUOTED = None


def stage1_fields(text, vocab, nouns=(), sid="s1"):
    """Stage-1, the bootstrap kernel's field extraction (moved whole from
    compiler.tokenize_statement — the behavioral spec): the statement's field
    FACTS from its text against the supplied VOCABULARY (the classLit
    population, hoisted ONCE per sweep by the caller — the per-statement
    reducer fetch was the fleet's 10-25-minute compile pocket). A Trailing
    Marker must trail; Role References are known-noun occurrences; a quoted
    token is a Literal Role; recognizer tokens and Role References never fire
    INSIDE a quoted literal; structural punctuation outside literals is the
    prose tell. Returns [(field_ft, (sid, value)), …]."""
    import re
    global _S1_QUOTED_SPAN, _S1_QUOTED
    if _S1_QUOTED_SPAN is None:
        _S1_QUOTED_SPAN = re.compile(r"'[^']*'")
        _S1_QUOTED = re.compile(r"'([^']*)'")
    text = text.strip().rstrip(".")
    bare = _S1_QUOTED_SPAN.sub(lambda m: " " * len(m.group(0)), text)
    out = []
    for (ftb, lit) in sorted(vocab, key=lambda p: -len(p[1])):
        if not re.search(r"(?<![A-Za-z])" + re.escape(lit) + r"(?![A-Za-z])",
                         bare, re.IGNORECASE):
            continue
        if ftb == "Statement_has_Trailing_Marker" \
                and not bare.rstrip().lower().endswith(lit.lower()):
            continue
        out.append((ftb, (sid, lit)))
    for n in nouns:
        if re.search(r"(?<![A-Za-z])" + re.escape(n) + r"(?![A-Za-z])", bare):
            out.append(("Statement_has_Role_Reference", (sid, n)))
    quoted = _S1_QUOTED.findall(text)
    if quoted:
        out.append(("Statement_has_Literal_Role", (sid, quoted[0])))
    for mark in (",", "(", ")", ": "):
        if mark in bare:
            out.append(("Statement_has_Prose_Punctuation", (sid, mark)))
            break
    return out


def _stage1_fields_impl(mu):
    """Stage-1 at the lex boundary (beside lex/implode/slug — the tokenizer
    stratum): ⟨text, vocab, nouns, sid⟩ → the field-fact rows. The
    implementation is host regex registered against the prim name — the
    operating rule (Samuel, 2026-07-07): a performant implementation proven
    to the interface by the contract tests (test_stage1_canon); a canonical
    composition is not owed at the boundary, exactly as lex itself."""
    from . import defs as _d
    import pyarest.lam as L

    def g(o):
        it = _d._items(L._list(o))
        if len(it) != 4:
            return L.BOT
        text, sid = _d._aval(it[0]), _d._aval(it[3])
        if not isinstance(text, str) or not isinstance(sid, str):
            return L.BOT
        vocab = []
        for p in _d._items(L._list(it[1])):
            pi = _d._items(L._list(p))
            if len(pi) >= 2:
                vocab.append((_d._aval(pi[0]), _d._aval(pi[1])))
        nouns = tuple(_d._aval(n) for n in _d._items(L._list(it[2])))
        from .lam import to_lam
        return to_lam(tuple((ft, tuple(r))
                            for (ft, r) in stage1_fields(text, vocab,
                                                         nouns, sid)))
    return g


def _register_lex_boundary():
    from .defs import register
    register("lex", _lex_impl)
    register("implode", _implode_impl)
    register("slug", _slug_impl)
    register("stage1_fields", _stage1_fields_impl)


_register_lex_boundary()


def table_columns(partition, table):
    """The fact types absorbed into `table`, in declaration order; column j of the 3NF row
    ⟨key, v1, v2, …⟩ holds the (1+j)th entry's value. The meaning is the canonical
    system:table_columns (shared/system.canon), the RMAP partition pairs filtered to the
    target table with its own entity fact type excluded, then projected to the fact type.
    This host binding applies that def through the reducer, so every host reads one
    definition; the twin is pinned in test_shared_builders."""
    from .lam import atom as A, to_lam, from_lam
    from .reduce import apply as _apply
    pairs = to_lam(tuple(partition.items()))
    return list(from_lam(_apply(_apply(A("system:table_columns"), A(table)), pairs)))


def row_resolve(col, width, unary=False):
    """resolve for an entity-cell write: ⟨I, row⟩ → row′, where I = ⟨key, value⟩
    and the cell holds the entity's 3NF row (a fresh entity gets holes, the
    default object #). A conflicting functional write makes the column ⊥, the
    row collapses (§11.2.1), and the step's transition rule refuses it
    atomically: absorption makes the UC structural. The canonical
    system:row_resolve applied to ⟨col, width, unary?⟩."""
    from .reduce import apply as _apply
    return _apply(A("system:row_resolve"),
                  _S(A(col), A(width), A("T" if unary else "F")))


def create_routed(D, ft, fact, partition, machine=None, mealy_obj=None, validate_obj=None):
    """Route a create through the RMAP partition (spec §4.4: the partition IS the layout).
    An absorbed fact type writes the entity's own cell `table:key`, the write unit of
    Def. iso, updating its column of the row; an own-table fact type creates into its
    per-fact-type cell unchanged. The table's index cell records the key IN THE SAME
    STEP (index_cell rides the commit chain like the machine slot), so a refused write
    leaves the index untouched. A machine (the row form) advances within the routed
    step; a validate (row_validate: step 5's constraint mapping) refuses within it."""
    from . import ast
    from .lam import from_lam
    table = partition.get(ft, ft)
    if table == ft:
        return ast.run(fact, D, cell_name=ft, machine=machine, mealy_obj=mealy_obj,
                       validate_obj=validate_obj)
    cols = table_columns(partition, table)
    col = 2 + cols.index(ft)
    key = from_lam(fact)[0]
    unary = max((r[2] for r in _pop_rows(D, "role") if len(r) >= 3 and r[1] == ft),
                default=2) == 1
    return ast.run(fact, D, cell_name=f"{table}:{key}",
                   resolve_obj=row_resolve(col, 1 + len(cols), unary),
                   machine=machine, mealy_obj=mealy_obj, validate_obj=validate_obj,
                   index_cell=table, append_cell=ft)


def ft_view(D, ft, partition):
    """Reassemble an absorbed fact type's ⟨key, value⟩ population from the
    entity cells, through the same canonical expression the pipeline reads
    (system:ftpop_absorbed via ftpop_expr); an own-table fact type reads its
    own cell. Unary fact types reshape their boolean column back host-side."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    table = partition.get(ft, ft)
    if table == ft:
        return set(from_lam(_ap(ast.FetchPop(ft), D)))
    unary = max((r[2] for r in _pop_rows(D, "role") if len(r) >= 3 and r[1] == ft),
                default=2) == 1
    pairs = set(from_lam(_ap(ftpop_expr(ft, partition), D)))
    if unary:
        return {(k,) for (k, v) in pairs if v == "T"}         # the boolean column, back
    return pairs


def install_entity_cells(D, noun, rows):
    """Each entity its own cell (whitepaper §Cells; the TS engine's one-DO-per-cell):
    ⟨CELL, noun:id, row⟩ per 3NF row, addressed as the reference engine addresses it and
    the write unit Def. iso isolates."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import to_lam
    import pyarest.lam as L
    for row in rows:
        key = f"{noun}:{row[0]}"
        D = _ap(ast.Store(key), L.SEQ(L.CONS(to_lam(tuple(row)))(L.CONS(D)(L.NIL))))
    return D


def _status_rows(D, noun):
    """The noun's live status population: the "is currently in Status" fact
    type read off its RMAP column (ft_view). The one seam every status reader
    goes through, dual with create_spec's write side; a noun with no machine
    (no smStatusFt marker) has no statuses."""
    status_ft = next((r[1] for r in _pop_rows(D, "smStatusFt")
                      if len(r) >= 2 and r[0] == noun), None)
    if status_ft is None:
        return []
    return sorted(ft_view(D, status_ft, rmap_partition(D)))


def _plain_rows(facts):
    """Rows as tuples all the way down (json lists arrive from the event
    sink; the fold passes tuples already)."""
    out = []
    for f in facts:
        out.append(tuple(f) if isinstance(f, (list, tuple)) else (f,))
    return out


def bulk_absorbed_install(D, part, table, ft, facts, replace_keys=False):
    """The batch install of an absorbed fact type's rows: each <key, value>
    lands on the entity's 3NF row (fresh rows hole-padded, the row_resolve
    shape), the key joins the table index, and the fact type's ** view cache
    unions -- one from_lam/to_lam pass instead of N routed creates. A unary
    absorbed fact type's <key> sets its boolean column to T. replace_keys
    makes the write an OVERWRITE: the installed keys' stale view-cache rows
    are pruned instead of unioned (the machine fold's semantics -- one
    status per entity)."""
    from .lam import from_lam as _fl, to_lam as _tl
    cols = table_columns(part, table)
    col = 2 + cols.index(ft)
    width = 1 + len(cols)
    unary = max((r[2] for r in _pop_rows(D, "role")
                 if len(r) >= 3 and r[1] == ft), default=2) == 1
    cells_l = list(_fl(D))
    idx = {c[1]: i for i, c in enumerate(cells_l)
           if isinstance(c, tuple) and len(c) >= 3 and c[0] == "CELL"}

    def setcell(name, val):
        if name in idx:
            cells_l[idx[name]] = ("CELL", name, val)
        else:
            idx[name] = len(cells_l)
            cells_l.append(("CELL", name, val))

    tbl = list(cells_l[idx[table]][2]) if table in idx else []
    keys = {r[0] for r in tbl if r}
    rows = _plain_rows(facts)
    for r in rows:
        if not r:
            continue
        k = r[0]
        v = "T" if unary else (r[1] if len(r) >= 2 else "#")
        rc = f"{table}:{k}"
        row = list(cells_l[idx[rc]][2]) if rc in idx else []
        if not row:
            row = [k] + ["#"] * (width - 1)
        while len(row) < width:
            row.append("#")
        row[col - 1] = v
        setcell(rc, tuple(row))
        if k not in keys:
            keys.add(k)
            tbl.append((k,))
    setcell(table, tuple(tbl))
    view = {tuple(r) for r in (cells_l[idx[ft]][2] if ft in idx else ())}
    if replace_keys:
        installed = {r[0] for r in rows if r}
        view = {r for r in view if not r or r[0] not in installed}
    view |= {tuple(r) for r in rows}
    setcell(ft, tuple(_rowsort(view)))
    return _tl(tuple(cells_l))


def machine_fold(D):
    """Readings-carried machine events, folded at compile: instance facts of
    trigger fact types ARE the event stream when it arrives as readings (the
    tasks board's sm-migration class), and the write path's incremental fold
    never sees them — the promised "event-fold after compile" was missing
    (found 2026-07-08: 75 tasks wedged at init while their readings said
    finished). Per governed entity: start from the column's current status
    (else the machine's initial), fire the first fireable event in a fixed
    order, remove it, repeat until nothing fires — the machine itself orders
    the walk (readings carry no timestamps; a deterministic machine makes the
    greedy walk deterministic). Events invalid from every reachable status
    stay as rows, the write path's no-op semantics. One create per changed
    entity lands the final status on the RMAP column (replay_entries' commit
    convention), so recompiles re-derive the same statuses from scratch."""
    from .lam import to_lam, atom as A
    from .reduce import apply as _apply
    triples = sm_triples(D)
    if not triples:
        return D
    trig_fts = sorted({r[1] for r in _pop_rows(D, "smTrigger")
                       if len(r) >= 2})
    initials = {r[1]: r[0] for r in _pop_rows(
        D, "Status_is_initial_in_State_Machine_Definition") if len(r) >= 2}
    status_fts = {r[0]: r[1] for r in _pop_rows(D, "smStatusFt")
                  if len(r) >= 2}
    machines = {r[1]: r[0] for r in _pop_rows(D, "smDef") if len(r) >= 2}
    gov = {r[0]: r[1] for r in _pop_rows(D, "governedBy") if len(r) >= 2}
    events = {}
    for ft in trig_fts:
        noun = _governed_player(D, ft)
        if noun is None:
            continue
        pos = next((r[2] for r in _pop_rows(D, "role")
                    if len(r) >= 4 and r[1] == ft and r[3] == noun), None)
        if pos is None:
            continue
        for row in _pop_rows(D, ft):
            if len(row) >= pos and row[pos - 1] not in ("", "φ"):
                events.setdefault((noun, row[pos - 1]), []).append(ft)
    part = rmap_partition(D)
    current = {}
    for noun in sorted({n for (n, _e) in events}):
        sft = status_fts.get(gov.get(noun, noun))
        if sft is None:
            continue
        for row in ft_view(D, sft, part):
            if isinstance(row, tuple) and len(row) >= 2:
                current[(noun, row[0])] = row[1]
    changed = []
    for (noun, e), evs in sorted(events.items()):
        sft = status_fts.get(gov.get(noun, noun))
        if sft is None:
            continue
        m = machines.get(gov.get(noun, noun), machines.get(noun))
        start = current.get((noun, e), initials.get(m))
        cur, evs = start, sorted(evs)
        fired_any, fired = False, True
        while fired and evs:
            fired = False
            for i, ev in enumerate(evs):
                to = next((t for (f, g, t) in triples
                           if g == ev and f == cur), None)
                if to is not None:
                    cur, fired, fired_any = to, True, True
                    evs.pop(i)
                    break
        # write iff the machine RAN for this entity — a round-trip back
        # to the initial still materializes (the write path would have);
        # an entity whose every event is unfireable stays untouched
        if fired_any and cur != current.get((noun, e)):
            changed.append((sft, e, cur))
    # SM init, the design's second half ("SM init covers the rest"):
    # every governed entity with no status row materializes the
    # machine's initial. The entity source is the noun's own table
    # UNIONED with the role-1 keys of every fact type the noun heads —
    # a fresh compile carries pops before any table cell materializes,
    # and an entity is an entity by playing a fact (an unfireable
    # event still evidences its player).
    written = {(s, e) for s, e, _c in changed}
    for noun, m in sorted(machines.items()):
        sft = status_fts.get(noun)
        init = initials.get(m)
        if sft is None or init is None:
            continue
        have = {r[0] for r in ft_view(D, sft, part)
                if isinstance(r, tuple) and r}
        keys = {r[0] for r in _pop_rows(D, noun) if r}
        for r in _pop_rows(D, "role"):
            if len(r) >= 4 and r[2] == 1 and r[3] == noun and r[1] != sft:
                keys |= {x[0] for x in _pop_rows(D, r[1]) if x}
        for k in sorted(keys, key=str):
            if (k and k not in ("", "φ") and k not in have
                    and (sft, k) not in written):
                written.add((sft, k))
                changed.append((sft, k, init))
    by_sft = {}
    for sft, e, cur in changed:
        by_sft.setdefault(sft, []).append((e, cur))
    for sft, rows in sorted(by_sft.items()):
        table = part.get(sft, sft)
        if table == sft:
            # own-table status (a machine before status_facts absorbs it):
            # union-overwrite the pop directly
            from . import ast as _ast
            keys = {r[0] for r in rows}
            keep = [tuple(r) for r in _pop_rows(D, sft)
                    if r and r[0] not in keys]
            D = _apply(_ast.Store(sft),
                       _S(to_lam(_rowsort(set(keep) | set(rows))), D))
        else:
            D = bulk_absorbed_install(D, part, table, sft, rows,
                                      replace_keys=True)
    return D


def sm_init_entity(D, ft, row):
    """The write path's SM init: a committed create can BIRTH a governed
    entity — a non-trigger fact whose role-1 key has no status row yet
    (fr-live-1's null status, 2026-07-08). The compile-time fold covers
    readings- and log-carried events wholesale; this covers ONE write for
    ONE entity: land the machine's initial on the status column through
    the same commit convention (no log append — the next compile's fold
    re-derives the same initial deterministically), then derive
    delta-scoped over the status fact type. Identity when the app has no
    machines, the fact plays no governed noun, or the entity already has
    a status — the apply path pays a few marker-pop reads and nothing
    else. Trigger facts stay create_routed's business (the machine
    advances within the routed step); this only seeds birth."""
    from .lam import to_lam
    machines = {r[1]: r[0] for r in _pop_rows(D, "smDef") if len(r) >= 2}
    if not machines:
        return D
    row = tuple(row)
    if not row or row[0] in ("", "φ"):
        return D
    noun = next((r[3] for r in _pop_rows(D, "role")
                 if len(r) >= 4 and r[1] == ft and r[2] == 1), None)
    if noun is None:
        return D
    gov = {r[0]: r[1] for r in _pop_rows(D, "governedBy") if len(r) >= 2}
    status_fts = {r[0]: r[1] for r in _pop_rows(D, "smStatusFt")
                  if len(r) >= 2}
    g = gov.get(noun, noun)
    sft = status_fts.get(g)
    m = machines.get(g, machines.get(noun))
    if sft is None or m is None or ft == sft:
        return D
    initials = {r[1]: r[0] for r in _pop_rows(
        D, "Status_is_initial_in_State_Machine_Definition") if len(r) >= 2}
    init = initials.get(m)
    if init is None:
        return D
    part = rmap_partition(D)
    key = row[0]
    if any(isinstance(r, tuple) and r and r[0] == key
           for r in ft_view(D, sft, part)):
        return D
    table = part.get(sft, sft)
    if table == sft:
        from . import ast as _ast
        from .reduce import apply as _apply
        keep = {tuple(r) for r in _pop_rows(D, sft)}
        D = _apply(_ast.Store(sft),
                   _S(to_lam(_rowsort(keep | {(key, init)})), D))
    else:
        D = bulk_absorbed_install(D, part, table, sft, [(key, init)],
                                  replace_keys=True)
    return run_rules(D, changed=[sft])


def moore_view(D, noun):
    """The Moore output function as a view: for each live instance whose status carries an
    emission, the ρ-application of the named definition to ⟨entity, status⟩ (outputs are
    ρ-applications; the definition resolves through D's own DEFS)."""
    from . import defs as _d
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam, atom as _A
    moore = {r[0]: r[1] for r in _pop_rows(D, "smMoore")}
    out = {}
    for row in _status_rows(D, noun):
        e, s = row[0], row[1]
        if s in moore:
            with _d.step(D):
                out[(e, s)] = from_lam(_ap(_A(moore[s]), to_lam((e, s))))
    return out


def process_table(D, noun):
    """The run queue as a VIEW (nothing is managed host-side): each state-machine
    instance whose status has outgoing transitions is a WAITING process, keyed to the
    trigger fact types it awaits (a subscription is a ρ-application not yet evaluated,
    Cor. stream); an instance whose status has none has terminated and leaves the table
    (links = φ, the paper's logical deletion)."""
    triples = sm_triples(D)
    out = {}
    for row in _status_rows(D, noun):
        e, s = row[0], row[1]
        awaits = tuple(tr for (f, tr, _t) in triples if f == s)
        if awaits:
            out[(e, s)] = awaits
    return out


def machine_step(trigger_ft, row_col=None):
    """The machine that runs IS the M-facts: one FFP object over ⟨statusPop, P″, D⟩ that
    reads the transitions (smFrom ⋈ smTrigger ⋈ smTo), the guards (smGuard), and the
    addressed entity's role position (role facts joined with the governed nouns, smDef
    plus the derived governedBy closure) from D INSIDE the reduction, then advances each
    entity whose trigger fact entered P″ with its guard satisfied. Numbers are selectors,
    so the runtime role position selects dynamically via the apply primitive. Editing M
    redirects this step with no rewiring; `trigger_ft` is the handler's compile-time
    identity, exactly as cell_name is for build_system. `row_col` selects the absorbed
    (3NF-row) fired form."""
    from .reduce import apply as _apply
    from .lam import to_lam
    rc = to_lam(()) if row_col is None else _S(to_lam(row_col))
    return _apply(A("system:machine_step"), _S(A(trigger_ft), rc))


def mealy_step(trigger_ft, row_col=None):
    """Mealy output on the SAME step: for each entity whose transition fires, the
    transition's named definition (smEmit, read from M in-step like everything else) is
    resolved by ρ from D's own cells (definitions are ordinary cells, §13.3.5) and
    applied to ⟨e, from, to⟩; the emissions ⟨⟨e, result⟩ …⟩ join the representation o.
    Silent transitions, absent definitions, and unfired machines emit nothing."""
    from .reduce import apply as _apply
    from .lam import to_lam
    rc = to_lam(()) if row_col is None else _S(to_lam(row_col))
    return _apply(A("system:mealy_step"), _S(A(trigger_ft), rc))


def _governed_player(D, ft):
    """The player of `ft` whose noun a machine governs (directly via smDef, or through
    the derived governedBy closure), and so whose status cell the trigger advances. The
    meaning is the canonical system:governed_player (shared/system.canon), a D-reader over
    the pair ⟨ft, D⟩: the union of the smDef nouns and the governedBy closure, then the
    first role of the fact type whose player lies in that union. This host binding applies
    that def through the reducer, so every host reads one definition; the twin is pinned
    in test_shared_builders."""
    from .lam import atom as A, from_lam
    from .reduce import apply as _apply
    r = from_lam(_apply(A("system:governed_player"), _S(A(ft), D)))
    return None if r == () else r


_AUTH_FT = "User_is_authorized_for_Operation_on_Resource"


def create(D, fact_type, fact, fuel=None, actor=None, operation="create"):
    """THE ORM-level entry: the caller names only the fact. Whether a machine runs is the
    ORM layer's business, read off M — when `fact_type` is some transition's trigger
    (smTrigger) and one of its players is governed (smDef plus the derived governedBy
    closure), the M-driven machine step and its Mealy emissions are attached to that
    player's status cell; how it runs is the AST layer's (Prop. onestep: the one
    transition, the trigger fact entering P IS the firing). Absorbed fact types route to
    their RMAP table, the machine taking the row form (fired = the trigger's column went
    non-hole on the addressed entity's own 3NF row)."""
    from . import ast
    part = rmap_partition(D)
    table = part.get(fact_type, fact_type)
    # authorization (the access module, when ingested): the actor must hold the derived
    # ⟨user, operation, resource⟩ where the resource is the RMAP table the write lands
    # in; refusal answers ⟨ERROR, unchanged D⟩. Absent module: ungoverned (graceful).
    if actor is not None and any(r and r[0] == _AUTH_FT for r in _pop_rows(D, "factType")):
        allowed = {tuple(r) for r in _pop_rows(D, _AUTH_FT)}
        if (actor, operation, table) not in allowed:
            return _S(A("ERROR"), D)
    return _create_from_spec(D, fact_type, fact, create_spec(D, fact_type, part), fuel)


def create_spec(D, fact_type, part=None):
    """The SCHEMA-determined create recipe for a fact type, stable across writes
    (the goal: full native for every part but lambda and defs, so create is a def
    the resident reduces, not host orchestration). Returns the objects and params
    create computes MINUS the fact and its key: the routing (table, absorbed
    column, width, unary), the value validate object, and the machine, mealy, and
    links objects, each a canonical lambda tree or None. Stored as create:<ft> at
    compile so any host builds the handler and reduces it natively on apply."""
    if part is None:
        part = rmap_partition(D)
    table = part.get(fact_type, fact_type)
    absorbed = table != fact_type
    row_col = 2 + table_columns(part, table).index(fact_type) if absorbed else None
    machine = mealy = links = None
    if any(r[1] == fact_type for r in _pop_rows(D, "smTrigger")):
        noun = _governed_player(D, fact_type)
        if noun is not None:
            role_pos = next((r[2] for r in _pop_rows(D, "role")
                             if len(r) >= 4 and r[1] == fact_type and r[3] == noun), None)
            # status(e) falls out of RMAP: the status fact type is absorbed as a
            # column on the machine's OBJECT TYPE (the smDef noun; a governed
            # subtype player reaches it through the governedBy closure), and the
            # machine reads and overwrites that column (⟨table, col, width⟩).
            # A machine without its status column is an incomplete model.
            gov = {r[0]: r[1] for r in _pop_rows(D, "governedBy") if len(r) >= 2}
            status_ft = next((r[1] for r in _pop_rows(D, "smStatusFt")
                              if len(r) >= 2 and r[0] == gov.get(noun, noun)), None)
            if status_ft is None or status_ft not in part:
                raise ValueError(
                    f"machine on {noun!r} without its status column: run "
                    "system.status_facts (then layout_cells) before create — "
                    "status(e) IS the '<Noun> is currently in Status' fact type")
            scols = table_columns(part, part[status_ft])
            status_target = (part[status_ft], 2 + scols.index(status_ft),
                             1 + len(scols))
            machine = (status_target, machine_step(fact_type, row_col), role_pos)
            mealy = mealy_step(fact_type, row_col)
            if not absorbed and role_pos is not None:
                from .lam import to_lam
                links = transitions_of(to_lam(sm_triples(D)), 2)
    spec = {"table": table, "absorbed": absorbed,
            "machine": machine, "mealy": mealy, "links": links}
    if absorbed:
        cols = table_columns(part, table)
        spec["col"] = 2 + cols.index(fact_type)
        spec["width"] = 1 + len(cols)
        spec["unary"] = max((r[2] for r in _pop_rows(D, "role")
                             if len(r) >= 3 and r[1] == fact_type), default=2) == 1
        spec["validate"] = row_validate(D, fact_type, part)
    return spec


def _create_from_spec(D, fact_type, fact, spec, fuel=None):
    """Build the create handler from the schema recipe and reduce it over the
    fact: an own-table fact type writes its own cell; an absorbed one writes the
    entity's cell table:key, the key read from the fact at write time (the one
    fact-dependent piece, the dynamic store the resident computes)."""
    from . import ast
    from .lam import from_lam
    machine, mealy = spec["machine"], spec["mealy"]
    if not spec["absorbed"]:
        return ast.run(fact, D, cell_name=fact_type, machine=machine,
                       mealy_obj=mealy, links_obj=spec["links"], fuel=fuel)
    key = from_lam(fact)[0]
    resolve = row_resolve(spec["col"], spec["width"], spec["unary"])
    return ast.run(fact, D, cell_name=f"{spec['table']}:{key}",
                   resolve_obj=resolve, machine=machine, mealy_obj=mealy,
                   validate_obj=spec["validate"], index_cell=spec["table"],
                   append_cell=fact_type, fuel=fuel)


def _absorbed_handler(spec, ft):
    """PHASE TWO, done: the fact-DEPENDENT create handler stored WHOLE. The
    nine-slot options record computes its cell name from the fact at reduce
    time — apply(cellkey, ⟨table, key⟩), key = N1(fact) of the operand
    P = ⟨fact, D⟩ — and every other slot is the spec's constant, so
    apply(ast:build_system(record(P)), P) reduces exactly the transition
    _create_from_spec wires host-side (routing, row_resolve, validate, the
    machine's status-column advance, index and append legs)."""
    from .lam import to_lam

    def K_(x):
        return _S(A("CONST"), x)

    def slot(v):
        return to_lam(()) if v is None else _S(v)

    resolve = row_resolve(spec["col"], spec["width"], spec["unary"])
    machine = spec["machine"]
    m = to_lam(()) if machine is None else _S(
        to_lam(machine[0]), machine[1], *(A(r) for r in machine[2:]))
    key = _S(A("COMP"), A(1), A(1))
    cellfn = _S(A("COMP"), A("apply"),
                _S(A("CONS"), K_(A("cellkey")),
                   _S(A("CONS"), K_(A(spec["table"])), key)))
    rec = _S(A("CONS"), cellfn,
             K_(slot(spec["validate"])), K_(slot(resolve)), K_(to_lam(())),
             K_(to_lam(())), K_(m), K_(slot(spec["mealy"])),
             K_(slot(A(spec["table"]))), K_(slot(A(ft))))
    build = _S(A("COMP"), A("apply"),
               _S(A("CONS"), K_(A("ast:build_system")), rec))
    return _S(A("COMP"), A("apply"), _S(A("CONS"), build, A("id")))


def create_handlers(D):
    """Store create:<ft> handler cells, the goal being full native for every part
    but lambda and defs: a create handler is a DEF the resident reduces over the
    fact, no host orchestration at write time. An own-table handler is
    fact-INDEPENDENT (fixed cell name) and stores build_system whole; an absorbed
    handler computes its cell name from the fact at reduce time
    (_absorbed_handler), so the resident serves BOTH natively off the cell's
    presence. Called at compile beside the layout and generator cells; recompile
    replaces the family."""
    from .lam import to_lam, from_lam
    from . import ast
    part = rmap_partition(D)
    cells = tuple(c for c in from_lam(D)
                  if not (isinstance(c, tuple) and len(c) >= 2
                          and str(c[1]).startswith("create:")))
    fresh = []
    for f in _pop_rows(D, "factType"):
        if not f:
            continue
        ft = f[0]
        spec = create_spec(D, ft, part)
        if spec["absorbed"]:
            handler = _absorbed_handler(spec, ft)
        else:
            handler = ast.build_system(cell_name=ft, machine=spec["machine"],
                                       mealy_obj=spec["mealy"],
                                       links_obj=spec["links"])
        fresh.append(("CELL", "create:" + ft, from_lam(handler)))
    return to_lam(cells + tuple(fresh))


def create_stamped(D, ft, fact, tx):
    """Bitemporal τ (Halpin §13.6): transaction time is when the SYSTEM records the fact
    — ⟨tx, …fact⟩ enters the ft@tx log beside the base fact; valid time is ordinary UoD
    data inside the fact itself. The platform's stream sequencer supplies tx: arrival
    order at a stream IS transaction time (writer model); the engine holds no clock."""
    from . import ast, defs as _d
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam
    import pyarest.lam as L
    res = create(D, ft, fact)
    o, D2 = _d._items(L._list(res))
    if from_lam(o) == "ERROR":
        return D2
    rows = tuple(tuple(r) for r in _pop_rows(D2, ft + "@tx")) + \
        ((tx,) + tuple(from_lam(fact)),)
    return _ap(ast.Store(ft + "@tx"), _S(to_lam(rows), D2))


def as_of(D, ft, tx):
    """The population as of transaction time `tx`, reconstructed from the τ log —
    Prop. onestep's order_τ audit view: Filter(tx′ ≤ tx), then project the fact."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    keep = T.Filter(_S(_COMP, A("le"), _S(_CONS, _1, _S(_CONST, A(tx)))))
    expr = _S(_COMP, _S(_ALPHA, A("tl")), keep, ast.FetchPop(ft + "@tx"))
    return {tuple(r) for r in from_lam(_ap(expr, D))}


def subscribe(D, sub_id, cells, def_name):
    """Cor. stream: a subscription IS a ρ-application that has not yet been evaluated
    against the current D — `def_name` names an ordinary definition (a cell, §13.3.5);
    the subscription facts record which cells it reads. The pending set is data."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import to_lam
    rows = tuple(tuple(r) for r in _pop_rows(D, "subscription")) + \
        tuple((sub_id, c, def_name) for c in cells)
    return _ap(ast.Store("subscription"), _S(to_lam(rows), D))


def _changed_closure(D, changed):
    """`changed` closed transitively through the rule graph: a changed cell wakes what
    reads it, and what those rules derive changes in turn (the frontier's derives hop,
    iterated). A sound over-approximation: waking a subscription whose cell did not in
    fact change merely re-evaluates the deferred ρ-application, which is its meaning."""
    reads = [(r[0], r[1]) for r in _pop_rows(D, "ruleReads") if len(r) >= 2]
    derives = {}
    for r in _pop_rows(D, "ruleDerives"):
        if len(r) >= 2:
            derives.setdefault(r[0], set()).add(r[1])
    out = set(changed)
    while True:
        grown = set(out)
        for (rule, ft) in reads:
            if ft in grown:
                grown |= derives.get(rule, set())
        if grown == out:
            return out
        out = grown


def wake(D, changed):
    """Evaluate every subscription due on `changed` (transitively through the rule
    graph): the deferred ρ-applications, now applied to the current D. Returns
    {subscription id: value}."""
    from . import defs
    from .reduce import apply as _ap
    from .lam import from_lam, atom as _A
    cl = _changed_closure(D, changed)
    due = {}
    for r in _pop_rows(D, "subscription"):
        if len(r) >= 3 and r[1] in cl:
            due[r[0]] = r[2]
    out = {}
    with defs.step(D):
        for sid, dname in due.items():
            out[sid] = from_lam(_ap(_A(dname), D))
    return out


def step_and_wake(D, fact_type, fact):
    """The commit path, wired (Cor. stream): one ORM-level create, the semi-naive
    derivation of the affected fragment, then the subscriptions due on what changed.
    Returns (⟨o, D′⟩, wakes); a refused step (ERROR) derives and wakes nothing."""
    from . import defs as _d
    from .lam import from_lam
    import pyarest.lam as L
    res = create(D, fact_type, fact)
    o, D2 = _d._items(L._list(res))
    if from_lam(o) == "ERROR":
        return res, {}
    changed = {fact_type}
    if any(r[1] == fact_type for r in _pop_rows(D2, "smTrigger")):
        noun = _governed_player(D2, fact_type)
        if noun is not None:
            # the machine advanced the governed object type's status column:
            # rules reading "<Noun> is currently in Status" re-derive
            gov = {r[0]: r[1] for r in _pop_rows(D2, "governedBy") if len(r) >= 2}
            sft = next((r[1] for r in _pop_rows(D2, "smStatusFt")
                        if len(r) >= 2 and r[0] == gov.get(noun, noun)), None)
            if sft is not None:
                changed.add(sft)
    D2 = run_rules(D2, changed=changed)
    return _S(o, D2), wake(D2, changed)


def ftpop_expr(ft, partition):
    """The fact type's population as one FFP expression over D, whatever the
    layout: an own-table fact type reads its cell; an absorbed one reassembles
    ⟨key, value⟩ through the index and the dynamic fetch (the canonical
    system:ftpop_absorbed applied to ⟨table, col⟩). The seam the RMAP plan
    recorded: scoped constraints read through the VIEW."""
    from . import ast
    from .reduce import apply as _apply
    table = partition.get(ft, ft)
    if table == ft:
        return ast.FetchPop(ft)
    col = 2 + table_columns(partition, table).index(ft)
    return _apply(A("system:ftpop_absorbed"), _S(A(table), A(col)))


def row_validate(D, ft, partition):
    """Step 5's constraint mapping (Halpin §10.3): schema constraints move with
    the partitioned layout. A value constraint on an absorbed fact type's value
    player checks the ROW's column on the routed write, holes skipped, the flag
    alethic per modality. The canonical system:row_validate applied to
    ⟨col, vc-name, alethic?⟩; the M-fact lookups stay host as this applier.
    None when nothing maps."""
    from .reduce import apply as _apply
    table = partition.get(ft, ft)
    if table == ft:
        return None
    col = 2 + table_columns(partition, table).index(ft)
    players = [r[3] for r in _pop_rows(D, "role") if len(r) >= 4 and r[1] == ft]
    vcs = {r[0]: r for r in _pop_rows(D, "valueConstraint") if len(r) >= 3}
    hits = [vcs[p] for p in players if p in vcs]
    if not hits:
        return None
    vt, _spec, modality = hits[0][0], hits[0][1], hits[0][2]
    return _apply(A("system:row_validate"),
                  _S(A(col), A(vt + "_vc"),
                     A("T" if modality == "alethic" else "F")))


def facts_about(D, entity):
    """Every fact mentioning `entity`, VERBALIZED: scan the flat fact cells for rows
    containing it and render each through its fact type's reading template (Prop.
    spec's verbalize direction, applied to instances). Returns (ft, row, sentence)."""
    from .lam import from_lam
    readings = {f[0]: f[1] for f in _pop_rows(D, "factType") if len(f) >= 2}
    players = {}
    for r in _pop_rows(D, "role"):
        if len(r) >= 4:
            players.setdefault(r[1], {})[r[2]] = r[3]
    out = []
    for c in from_lam(D):
        if not (isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"):
            continue
        rows = c[2]
        if not (isinstance(rows, tuple) and all(
                isinstance(r, tuple) and all(not isinstance(x, tuple) for x in r)
                for r in rows)):
            continue
        template = readings.get(c[1])
        for r in rows:
            if entity in r:
                if template and template.count("{") == len(r):
                    # NORMA instance verbalization keeps the role player's type name
                    filled = [f"{players.get(c[1], {}).get(i + 1, '')} '{v}'".strip()
                              for i, v in enumerate(r)]
                    sentence = template.format(*filled) + "."
                else:
                    sentence = f"{c[1]}{r}"
                out.append((c[1], r, sentence))
    return out


def describe(D, noun):
    """What the system can say about a noun, from its own M-facts (a read view in the
    ft_view style): kind, supertypes and subtypes, the fact types it plays roles in
    (with their reading templates and this noun's position), reference mode, the
    machine governing it (if any), and federation provenance."""
    readings = {f[0]: f[1] for f in _pop_rows(D, "factType") if len(f) >= 2}
    roles = [(r[1], r[2], readings.get(r[1], ""))
             for r in _pop_rows(D, "role") if len(r) >= 4 and r[3] == noun]
    return {
        "noun": noun,
        "kind": sorted({r[1] for r in _pop_rows(D, "instanceOf")
                        if len(r) >= 2 and r[0] == noun}),
        "supertypes": sorted({b for (a, b) in
                              (r[:2] for r in _pop_rows(D, "subtype") if len(r) >= 2)
                              if a == noun}),
        "subtypes": sorted({a for (a, b) in
                            (r[:2] for r in _pop_rows(D, "subtype") if len(r) >= 2)
                            if b == noun}),
        "roles": sorted(roles),
        "ref_mode": sorted({r[1] for r in _pop_rows(D, "refMode")
                            if len(r) >= 2 and r[0] == noun}),
        "machine": machine_for(D, noun),
        "federated_from": sorted({r[1] for r in _pop_rows(D, "federatedFrom")
                                  if len(r) >= 2 and r[0] == noun}),
    }


def finality_modality(D, noun, depth):
    """The writer model's hardening rule, read off M's finality facts: below the noun's
    declared depth k a violation reports DEONTICALLY (optimistic acceptance, V as the
    repair obligation); at or beyond k it refuses ALETHICALLY. An undeclared noun is
    final immediately. Nakamoto §11 quantifies any chosen k."""
    ks = {r[0]: r[1] for r in _pop_rows(D, "finality") if len(r) == 2}
    k = ks.get(noun, 0)
    return "deontic" if depth < k else "alethic"


def declare_sig(D, name, dom, cod):
    """Def. reg's ⟨dom, cod⟩ as M facts: defSig rows ⟨name, position, objectType⟩ with
    cod at position 0 — DatalogLB-style typed-predicate constraints on the boundary."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import to_lam
    rows = tuple(tuple(r) for r in _pop_rows(D, "defSig")) + \
        tuple((name, i + 1, t) for i, t in enumerate(dom)) + ((name, 0, cod),)
    return _ap(ast.Store("defSig"), _S(to_lam(rows), D))


def checked_apply(name):
    """The typed boundary application (Def. reg dom/cod): ⟨args, D⟩ applies
    registered `name` iff every argument at a declared dom position is an
    instance of its object type (membership in the type's index cell, read
    from D in-step), else the ERROR atom the transition rule refuses
    (§14.3.1). Undeclared names apply unchecked; a sig naming an absent type
    cell fails closed. The canonical system:checked_apply applied to name."""
    from .reduce import apply as _apply
    return _apply(A("system:checked_apply"), A(name))


def finiteness_check(D):
    """The static condition discharging Lemma finiteness' hypothesis: recursion through
    the rule dependency graph is admitted — heads are range-restricted by construction,
    so the fixpoint runs over a finite atom domain and terminates — but no dependency
    CYCLE may pass through value invention, a rule whose definition applies a registered
    (boundary, Cor. boundary) function and so can introduce individuals drawn from no
    stored population, unboundedly. Value invention is (a) anything registered beyond
    the formal base (bridges, cellkey, FFI — the boundary proper) and (b) the base's own
    value-constructing ops (arithmetic, length, dynamic apply), which mint new atoms just
    as surely. Acyclic invention stays admissible (a finite composition introduces
    finitely many individuals). Returns the offending rule names. Definitions are cells,
    so rule bodies are read from D like everything else."""
    from . import defs, prims
    from .lam import from_lam
    reads, derives = {}, {}
    for (r, ft) in _pop_rows(D, "ruleReads"):
        reads.setdefault(r, set()).add(ft)
    for (r, ft) in _pop_rows(D, "ruleDerives"):
        derives.setdefault(r, set()).add(ft)
    boundary = (set(defs._registered) - set(prims.BASE)) | {"+", "-", "*", "div", "length", "apply"}

    def _atoms(v):
        if isinstance(v, tuple):
            for x in v:
                yield from _atoms(x)
        elif isinstance(v, str):
            yield v

    cells = defs._cells_of(D)
    inventive = set()
    for r in set(reads) | set(derives):
        body = cells.get(r)
        if body is not None and any(a in boundary for a in _atoms(from_lam(body))):
            inventive.add(r)
    succ = {}
    for r in reads:                                          # ft-level dependency edges
        for src in reads[r]:
            succ.setdefault(src, set()).update(derives.get(r, ()))

    def _reaches(a, b):
        seen, stack = set(), [a]
        while stack:
            n = stack.pop()
            if n == b:
                return True
            if n not in seen:
                seen.add(n)
                stack.extend(succ.get(n, ()))
        return False

    return sorted(r for r in inventive
                  if any(_reaches(d, s) for d in derives.get(r, ()) for s in reads.get(r, ())))


def governance_rules(D):
    """Install the governedBy closure with the engine's own rule machinery: a noun is
    governed by the machine it is bound to, and by any machine governing a supertype.
    run_rules then derives the closure like any other rule."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import to_lam
    plans = (("governedBy_rule_base", ["smDef"], [2, 1]),
             ("governedBy_rule_step", ["subtype", "governedBy"], [1, 3]))
    atoms = []
    for (name, fts, head) in plans:
        D = _ap(ast.DefineIn(name, compile_rule(fts, head)), D)
        for i, ft in enumerate(fts):                         # semi-naive ~d variants
            D = _ap(ast.DefineIn(f"{name}~d{i + 1}", compile_rule_delta(fts, head, i)), D)
            atoms.append((name, i + 1, ft))
    derives = tuple(tuple(r) for r in _pop_rows(D, "ruleDerives")) + \
        (("governedBy_rule_base", "governedBy"), ("governedBy_rule_step", "governedBy"))
    D = _ap(ast.Store("ruleDerives"), _S(to_lam(derives), D))
    reads = tuple(tuple(r) for r in _pop_rows(D, "ruleReads")) + \
        (("governedBy_rule_base", "smDef"), ("governedBy_rule_step", "subtype"),
         ("governedBy_rule_step", "governedBy"))
    D = _ap(ast.Store("ruleReads"), _S(to_lam(reads), D))
    rows = tuple(tuple(r) for r in _pop_rows(D, "ruleAtom")) + tuple(atoms)
    return _ap(ast.Store("ruleAtom"), _S(to_lam(rows), D))


def status_facts(D):
    """Each governed Object Type gets its "is currently in Status" fact type (the
    machine's status fact, whitepaper Prop. onestep: status(e) is the
    transition-fold over the entity's events, materialized per RMAP). Generated
    through the ordinary reading path so the NAME is compiled, never hand-built,
    and functional (each entity is currently in at most one Status) so RMAP
    absorbs it as the status column. The marker smStatusFt ⟨Object Type, status
    fact type⟩ is EXTRACTED from the compiled result (the new fact type's role-1
    player), so the guarded step looks status(e) up rather than reconstruct a
    name. Runs before layout/partition so the column is laid out."""
    from . import forml, ast
    from .reduce import apply as _apply
    from .lam import to_lam
    nouns = [r[1] for r in _pop_rows(D, "smDef") if len(r) >= 2]
    if not nouns:
        return D
    values = {r[0] for r in _pop_rows(D, "instanceOf") if len(r) >= 2 and r[1] == "ValueType"}
    lines = [] if "Status" in values else ["Status is a value type."]
    for noun in nouns:
        lines.append(f"{noun} is currently in Status.")
        lines.append(f"Each {noun} is currently in at most one Status.")
    before = {r[0] for r in _pop_rows(D, "factType")}
    # context_from=D: the model's OWN declared types (Status included, when the
    # model declares it) resolve as role players — without it a model-declared
    # Status is unknown to this compile and the status fact type mints UNARY
    D, _rep = forml.compile_model("\n".join(lines) + "\n", D=D, context_from=D)
    role1 = {r[1]: r[3] for r in _pop_rows(D, "role") if len(r) >= 4 and r[2] == 1}
    # the marker is extracted by the fact type's READING identity, not by
    # newness: a model may declare its own "<Noun> is currently in Status"
    # fact type (the tasks board's status bridge does, so the rule catalog
    # resolves it), and the declared form IS the status column exactly as
    # the synthesized one would be
    templ = {f[0]: str(f[1]) for f in _pop_rows(D, "factType") if len(f) >= 2}
    have = {tuple(r[:2]) for r in _pop_rows(D, "smStatusFt") if len(r) >= 2}
    markers = tuple(
        (noun, ft) for noun in nouns
        for ft in sorted(templ)
        if role1.get(ft) == noun and "is currently in" in templ[ft]
        and (noun, ft) not in have)
    rows = tuple(tuple(r) for r in _pop_rows(D, "smStatusFt")) + markers
    return _apply(ast.Store("smStatusFt"), _S(to_lam(rows), D))




# =====================================================================
# Machines as VALUES fed into ONE lambda (merged from machine.py,
# 2026-07-04, the fewer-files push; Prop. onestep: machine = foldl
# transition). A machine is not code, it IS its transition relation, a
# value; `run` folds a transition value over inputs from an initial
# state, applying with the `apply` primitive. rmap and csdp are Stage-1
# seed values demonstrating the shape (Halpin 10.3 grouping, 3.2
# populate); the full processes are roadmap phases, authored M-resident
# and run by this same fold.
# =====================================================================
_3, _TL, _NULL, _NOT = A(3), A("tl"), A("null"), A("not")
_APPLY, _APNDR = A("apply"), A("apndr")
_TL, _NULL, _NOT, _APPLY, _APNDR, _EQ, _CONST = A("tl"), A("null"), A("not"), A("apply"), A("apndr"), A("eq"), A("CONST")

# ---- run: the one lambda. run:⟨t, ⟨acc0, inputs⟩⟩ = foldl(t, acc0, inputs). ----
# The state threads ⟨t, acc, remaining⟩ so the transition VALUE travels with the fold and is
# applied to ⟨acc, input⟩ each step via `apply`. One runner; the machine is the value `t`.
_input   = _S(_COMP, _1, _3)                                 # 1:(3:state) — the current input
_new_acc = _S(_COMP, _APPLY, _S(_CONS, _1, _S(_CONS, _2, _input)))   # apply:⟨t, ⟨acc, input⟩⟩
_new_rem = _S(_COMP, _TL, _3)                                # tl:(3:state)
_step    = _S(_CONS, _1, _new_acc, _new_rem)                 # ⟨t, acc', rem'⟩
_hasmore = _S(_COMP, _NOT, _NULL, _3)                        # remaining non-empty?
_loop    = _S(_WHILE, _hasmore, _step)
_init    = _S(_CONS, _1, _S(_COMP, _1, _2), _S(_COMP, _2, _2))       # ⟨t, acc0, inputs⟩
machine_run = _S(_COMP, _2, _loop, _init)                            # 2:(loop:(init:arg)) = the final acc
define("run", machine_run)


def run_machine(transition, acc0, inputs):
    """Fold a transition VALUE over `inputs` from `acc0` — via the one `run` lambda."""
    from .reduce import apply
    return apply(machine_run, _S(transition, _S(acc0, inputs)))


# ---- RMAP as a value (Halpin §10.3): the two grouping rules, as a transition relation. ----
# Over fact-type facts ⟨factType, objectType, kind⟩, RMAP assigns each fact type a table:
#   functional role  → grouped ON the object type   (rule 2)
#   compound UC      → its OWN table                 (rule 1)
# The transition emits ⟨tableKey, factType⟩; folding it over the schema is the mapping — and
# "the decomposition into atomic facts IS the relational mapping": each key becomes a cell.
_kind = _S(_COMP, _3, _2)                                    # kind of the fact-type fact (2:arg)
_ot   = _S(_COMP, _2, _2)                                    # its object type
_ft   = _S(_COMP, _1, _2)                                    # its fact type
_is_functional = _S(_COMP, _EQ, _S(_CONS, _kind, _S(_CONST, A("functional"))))
_table_key = _S(_COND, _is_functional, _ot, _ft)            # rule 2 → object type ; rule 1 → own
_entry = _S(_CONS, _table_key, _ft)                         # ⟨tableKey, factType⟩
rmap = _S(_COMP, _APNDR, _S(_CONS, _1, _entry))            # apndr:⟨acc, ⟨tableKey, factType⟩⟩
define("rmap", rmap)


# ---- CSDP as a value: another transition, run by the SAME lambda. ----
# The CSDP populate step (Halpin §3.2) folds elementary example facts into the schema's fact
# set. Here that step is `apndr` (accumulate each verbalized fact) — a different value into `run`.
csdp = _APNDR
define("csdp", csdp)
