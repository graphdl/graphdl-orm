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
    §13.3.4). The canonical builder applied to the name (shared/ast.py)."""
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
    `machine=(status_cell, sm_obj)` is wired, the noun's status cell: the trigger fact entering
    P advances the machine within the SAME step (Prop. onestep), atomically with the commit.
    With `mealy_obj` (same input shape as sm_obj) the fired transitions' Mealy emissions are
    appended to the representation o as its last part. With `index_cell` (the routed-write
    case) the table's key index records I's key in the SAME commit chain, so refusal leaves
    the index untouched and re-writes stay deduplicated. Commits iff the alethic flag is
    false."""
    from .lam import to_lam

    def slot(v):
        return to_lam(()) if v is None else _S(v)

    m = to_lam(()) if machine is None else _S(A(machine[0]), machine[1],
                                              *(A(r) for r in machine[2:]))
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
