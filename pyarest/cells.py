"""Cells and the state store (Backus §13.3.4): D as a sequence of ⟨CELL,name,contents⟩."""
from .objects import Atom, Seq, seq, BOTTOM, DEFAULT, is_seq

CELL = Atom("CELL")


def cell(name, contents):
    """A cell triple ⟨CELL, name, contents⟩."""
    return seq(CELL, name, contents)


def _is_cell_named(x, name):
    return is_seq(x) and len(x.items) == 3 and x.items[0] == CELL and x.items[1] == name


def fetch(name, store):
    """↑n : contents of the first cell named n, else DEFAULT."""
    if not is_seq(store):
        return BOTTOM
    for x in store.items:
        if _is_cell_named(x, name):
            return x.items[2]
    return DEFAULT


def pop(name, store):
    """Remove the first cell named n."""
    out, removed = [], False
    for x in store.items:
        if (not removed) and _is_cell_named(x, name):
            removed = True
            continue
        out.append(x)
    return Seq(tuple(out))


def purge(name, store):
    """Remove all cells named n."""
    return Seq(tuple(x for x in store.items if not _is_cell_named(x, name)))


def store_(name, contents, store):
    """↓n : ⟨x, D⟩ → prepend a fresh cell named n (shadowing older ones)."""
    return Seq((cell(name, contents),) + pop(name, store).items)
