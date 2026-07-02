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
_STUB_VALIDATE = _S(_CONS, _1, _S(_CONST, PHI), _S(_CONST, A("F")))


def cell(name, contents):
    """A cell ⟨CELL, name, contents⟩ (Backus §13.3.4)."""
    return _S(CELL, A(name), contents)


def _named(name):
    """predicate on a cell: its name (role 2) equals `name`."""
    return _S(_COMP, _EQ, _S(_CONS, _2, _S(_CONST, A(name))))


def Fetch(name):
    """↑name — contents (role 3) of the first cell named `name`, else # (Backus §13.3.4)."""
    from .theta import Filter
    found = Filter(_named(name))
    return _S(_COND, _S(_COMP, _NULL, found),
              _S(_CONST, DEFAULT),                           # no such cell ⇒ # (unaddressable)
              _S(_COMP, _3, _1, found))                      # else contents of the first match


def FetchPop(name):
    """The create pipeline's view of a cell as a POPULATION: ↑name, with an absent cell an
    empty population — the fresh-cell default is the pipeline's explicit choice (a COND on
    #), never a change to ↑'s meaning."""
    f = Fetch(name)
    is_absent = _S(_COMP, _EQ, _S(_CONS, f, _S(_CONST, DEFAULT)))
    return _S(_COND, is_absent, _S(_CONST, PHI), f)


def Pop(name):
    """(pop n) — remove the FIRST cell named `name`, preserving deeper ones (§13.3.4: cells
    of one name form a LIFO stack; pop and purge are distinct operators). A WHILE-fold over
    ⟨removed?, acc, rest⟩ standing in for Backus's recursive definition."""
    head = _S(_COMP, _1, _3)
    hit = _S(_COMP, A("and"), _S(_CONS,
              _S(_COMP, _EQ, _S(_CONS, _1, _S(_CONST, A("F")))),      # not yet removed
              _S(_COMP, _named(name), head)))                          # and head is the cell
    take = _S(_CONS, _S(_CONST, A("T")), _2, _S(_COMP, A("tl"), _3))  # drop it, flag removed
    keep = _S(_CONS, _1, _S(_COMP, A("apndr"), _S(_CONS, _2, head)), _S(_COMP, A("tl"), _3))
    loop = _S(A("WHILE"), _S(_COMP, _NOT, _NULL, _3), _S(_COND, hit, take, keep))
    init = _S(_CONS, _S(_CONST, A("F")), _S(_CONST, PHI), A("id"))    # ⟨F, φ, D⟩
    return _S(_COMP, _2, loop, init)


def Purge(name):
    """(purge n) — remove ALL cells named `name` (§13.3.4's other operator)."""
    from .theta import Filter
    return Filter(_S(_COMP, _NOT, _named(name)))


def Store(name):
    """↓name — ⟨x, D⟩ → (push n):⟨x, (pop n):D⟩ (§13.3.4 verbatim): replace the TOP of the
    stack named `name`; deeper same-named cells survive. Fetch still reads the top."""
    make = _S(_CONS, _S(_CONST, CELL), _S(_CONST, A(name)), _1)   # ⟨x,D⟩ → ⟨CELL, name, x⟩
    return _S(_COMP, _APNDL, _S(_CONS, make, _S(_COMP, Pop(name), _2)))


def DefineIn(name, obj):
    """D → D′ with the definition stored as an ORDINARY cell ⟨CELL, name, obj⟩ of D by ↓name
    (Backus §13.3.5: such a cell has the same effect as Def name ≡ ρobj). Definitions travel
    with the store: self-modification is a step, a tenant's DEFS is its own, and mu resolves
    the name only within steps bound to this store (Prop. tenant / Cor. closure)."""
    return _S(_COMP, Store(name), _S(_CONS, _S(_CONST, obj), A("id")))       # D → ↓name:⟨obj, D⟩


def build_system(validate_obj=None, cell_name="FILE", resolve_obj=None, derive_obj=None, links_obj=None,
                 machine=None, mealy_obj=None):
    """The transition create_cell:⟨I, D⟩ → ⟨⟨P'',V⟩, D'⟩ over one cell, wired with a schema's
    validate (and optionally its resolve/derive). It touches only `cell_name` — plus, when
    `machine=(status_cell, sm_obj)` is wired, the noun's status cell: the trigger fact entering
    P advances the machine within the SAME step (Prop. onestep), atomically with the commit.
    With `mealy_obj` (same input shape as sm_obj) the fired transitions' Mealy emissions are
    appended to the representation o as its last part. Commits iff the alethic flag is false."""
    validate_obj = validate_obj if validate_obj is not None else _STUB_VALIDATE
    from .theta import Filter, member
    # fact-as-function: a population is a set, so the default resolve is append-if-absent
    # and re-assertion is the identity (at-least-once delivery is free for asserts)
    resolve_stage = resolve_obj if resolve_obj is not None else _S(_COND, member, _2, _APNDL)
    derive_stage = derive_obj if derive_obj is not None else A("id")
    P = _S(_COMP, FetchPop(cell_name), _2)                   # ⟨I,D⟩ → the cell's population
    resolved = _S(_COMP, resolve_stage, _S(_CONS, _1, P))    # resolve:⟨I, P⟩ = P'
    derived = _S(_COMP, derive_stage, resolved)              # derive:P' = P''
    # validate sees ⟨P'', D⟩: cell-local constraints read P''; scoped (cross-cell) ones
    # fetch sibling cells from the frozen D (audit C3 — no family drops from enforcement).
    # I rides along so links can address the entity the input names (Thm. hateoas).
    valDI = _S(_CONS, _S(_COMP, validate_obj, _S(_CONS, derived, _2)), _2, _1)  # ⟨⟨P'',V,flag⟩, D, I⟩
    P2 = _S(_COMP, _1, _1)                                   # P''  from ⟨val,D,I⟩
    V = _S(_COMP, _2, _1)                                    # V    from ⟨val,D,I⟩
    snew = entity_role = None
    if machine is not None:                                  # the machine advances in the SAME step:
        status_cell, sm_obj, *rest = machine                 # status′ = sm:⟨status, P″, D⟩, committed
        entity_role = rest[0] if rest else None              # with the fact — or neither (atomic);
        spop = _S(_COMP, FetchPop(status_cell), _2)          # D rides along so GUARDS can fetch
        snew = _S(_COMP, sm_obj, _S(_CONS, spop, P2, _2))    # their (possibly derived) fact type
    if links_obj is None:
        parts = [P2, V]
    else:
        links_in = P2
        if snew is not None and entity_role is not None:
            # the representation's controls come from the entity's POST-step status: the
            # σ(1 = e)(status′) singleton, e named by the input fact at entity_role —
            # "after which the representation offers ship and no longer place" (§1)
            e = _S(_COMP, A(entity_role), _3)                # the addressed entity, from I
            match_e = _S(_COMP, _EQ, _S(_CONS, _S(_COMP, _1, _1), _2))    # ⟨⟨e?,s⟩, e⟩
            links_in = _S(_COMP, _S(A("ALPHA"), _1), Filter(match_e), _DISTR, _S(_CONS, snew, e))
        parts = [P2, V, _S(_COMP, links_obj, links_in)]
    if mealy_obj is not None and machine is not None:        # Mealy: the fired transitions'
        parts.append(_S(_COMP, mealy_obj, _S(_CONS, spop, P2, _2)))   # emissions, last part of o
    o = _S(_CONS, *parts)                                    # o = ⟨P'', V⟩ or ⟨P'', V, links⟩ (hateoas)
    commit = _S(_COMP, Store(cell_name), _S(_CONS, P2, _2))  # ↓cell:⟨P'', D⟩
    if snew is not None:
        commit = _S(_COMP, Store(machine[0]), _S(_CONS, snew, commit))
    d_new = _S(_COND, _S(_COMP, _3, _1), _2, commit)         # alethic offender? D : commit
    return _S(_COMP, _S(_CONS, o, d_new), valDI)


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
        machine=None, mealy_obj=None, fuel=None):
    """One AST transition: mu(create_cell:⟨input, D⟩) = ⟨o, D'⟩, with D's OWN definitions in
    scope for the whole step (defs.step — frozen, Backus §14.6). Without a validate it commits
    (V = φ); with validate_of it refuses to commit on an alethic violation; with links_obj the
    representation o carries its HATEOAS links (Thm. hateoas); with machine=(status_cell, sm_obj
    [, entity_role]) the trigger fact advances the noun's machine in this same step (Prop.
    onestep) — and given the entity_role, links_obj is fed the entity's POST-step status, so
    the returned representation offers exactly the next actions (§1: ship, no longer place)."""
    from . import defs
    handler = build_system(validate_obj, cell_name, resolve_obj, derive_obj, links_obj, machine, mealy_obj)
    with defs.step(D, fuel):
        return _transition(apply(handler, _S(input_fact, D)), D)


# ============================ eq. sys — the whole system as one lambda =========
# SYSTEM : ⟨⟨entity, op⟩, D⟩  →  (rho(↑entity : D)) : ⟨op, D⟩         (the paper's eq. sys)
# The entire running engine is ONE lambda applied to values: D carries every entity's handler
# as a cell (a value); a command names an entity and an operation; the transition fetches that
# entity's handler FROM D (by runtime name), reflects it with rho, and applies it to ⟨op, D⟩.
# An address naming no cell of D fetches # — and #:x reduces to ⊥, so wrong-tenant access is
# not forbidden but impossible (Prop. tenant: isolation = preservation of addressability under ↑).

# DynFetch : ⟨name, D⟩ → contents of the first cell of D named `name` (a runtime value), else #.
_name_pairs = _S(_COMP, _DISTR, _S(_CONS, _2, _1))          # ⟨name, D⟩ → ⟨⟨cell, name⟩ …⟩
_cell_named = _S(_COMP, _EQ, _S(_CONS, _S(_COMP, _2, _1), _2))   # ⟨cell, name⟩ → (name of cell) = name?
def _dyn_hits():
    from .theta import Filter
    return _S(_COMP, Filter(_cell_named), _name_pairs)
def _DynFetch():
    hits = _dyn_hits()
    return _S(_COND, _S(_COMP, _NULL, hits),
              _S(_CONST, DEFAULT),                           # no such cell ⇒ # (unaddressable)
              _S(_COMP, _3, _1, _1, hits))                   # else contents of the first match's cell

def DynFetch():
    """The dynamic fetch expression over ⟨name, D⟩: contents of the first cell of D whose
    name equals the runtime value `name`, else # (the public form of eq. sys's fetch)."""
    return _DynFetch()


# SYSTEM : ⟨⟨entity, op⟩, D⟩ → apply:⟨↑entity:D, ⟨op, D⟩⟩
def _SYSTEM():
    handler = _S(_COMP, _DynFetch(), _S(_CONS, _S(_COMP, _1, _1), _2))   # ↑entity:D
    op_D = _S(_CONS, _S(_COMP, _2, _1), _2)                              # ⟨op, D⟩
    return _S(_COMP, _APPLY, _S(_CONS, handler, op_D))
SYSTEM = _SYSTEM()


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
