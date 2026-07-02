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
_COMP, _CONS, _CONST, _COND = A("COMP"), A("CONS"), A("CONST"), A("COND")
_1, _2, _3 = A(1), A(2), A(3)
_APNDL, _NULL, _NOT, _EQ = A("apndl"), A("null"), A("not"), A("eq")


def cell(name, contents):
    """A cell ⟨CELL, name, contents⟩ (Backus §13.3.4)."""
    return _S(CELL, A(name), contents)


def _named(name):
    """predicate on a cell: its name (role 2) equals `name`."""
    return _S(_COMP, _EQ, _S(_CONS, _2, _S(_CONST, A(name))))


def Fetch(name):
    """↑name — contents (role 3) of the first cell named `name`, else φ (a fresh cell)."""
    from .theta import Filter
    found = Filter(_named(name))
    return _S(_COND, _S(_COMP, _NULL, found),
              _S(_CONST, PHI),                               # no such cell ⇒ empty population
              _S(_COMP, _3, _1, found))                      # else contents of the first match


def Store(name):
    """↓name — ⟨x, D⟩ → D with the cell named `name` set to x (purge the old, prepend fresh)."""
    from .theta import Filter
    purge = Filter(_S(_COMP, _NOT, _named(name)))            # keep cells NOT named `name`
    make = _S(_CONS, _S(_CONST, CELL), _S(_CONST, A(name)), _1)   # ⟨x,D⟩ → ⟨CELL, name, x⟩
    return _S(_COMP, _APNDL, _S(_CONS, make, _S(_COMP, purge, _2)))


def build_system(validate_obj, cell_name="FILE", resolve_obj=None, derive_obj=None):
    """The transition create_cell:⟨I, D⟩ → ⟨⟨P'',V⟩, D'⟩ over one cell, wired with a schema's
    validate (and optionally its resolve/derive). It touches only `cell_name`, so distinct
    entities' handlers are isolated. Commits P'' iff the alethic flag is false."""
    resolve_stage = resolve_obj if resolve_obj is not None else _APNDL
    derive_stage = derive_obj if derive_obj is not None else A("id")
    P = _S(_COMP, Fetch(cell_name), _2)                      # ⟨I,D⟩ → the cell's population
    resolved = _S(_COMP, resolve_stage, _S(_CONS, _1, P))    # resolve:⟨I, P⟩ = P'
    derived = _S(_COMP, derive_stage, resolved)              # derive:P' = P''
    valD = _S(_CONS, _S(_COMP, validate_obj, derived), _2)   # ⟨⟨P'',V,flag⟩, D⟩
    P2 = _S(_COMP, _1, _1)                                   # P''  from ⟨val,D⟩
    V = _S(_COMP, _2, _1)                                    # V    from ⟨val,D⟩
    o = _S(_CONS, P2, V)                                     # o = ⟨P'', V⟩
    commit = _S(_COMP, Store(cell_name), _S(_CONS, P2, _2))  # ↓cell:⟨P'', D⟩
    d_new = _S(_COND, _S(_COMP, _3, _1), _2, commit)         # alethic offender? D : commit
    return _S(_COMP, _S(_CONS, o, d_new), valD)


def run(input_fact, D, validate_obj=None, cell_name="FILE", resolve_obj=None, derive_obj=None):
    """One AST transition: mu(create_cell:⟨input, D⟩) = ⟨⟨P'', V⟩, D'⟩. Without a validate it
    commits (V = φ); with validate_of(constraints) it refuses to commit on an alethic violation."""
    stub = _S(_CONS, A("id"), _S(_CONST, PHI), _S(_CONST, A("F")))   # V = φ, flag F ⇒ always commits
    handler = build_system(validate_obj if validate_obj is not None else stub,
                           cell_name, resolve_obj, derive_obj)
    return apply(handler, _S(input_fact, D))
