"""DEFS — the state D (Backus §14): a sequence of cells, here a Church list.
A cell is the pair ⟨name, contents⟩. `fetch` is Backus's naming function
(§13.3.4): a fold whose branch is chosen by a Church boolean (numeral
equality). Pure λ — no host comparison, no `if`.
"""
from . import lam as L
from .objects import numeral_of, DEFAULT

_D = [L.NIL]                          # current state D (a Church list of cells)

cell = lambda name: lambda contents: L.PAIR(name)(contents)


def define(name_atom, contents):      # install a cell (reader boundary)
    _D[0] = L.CONS(cell(name_atom)(contents))(_D[0])


def defs():
    return _D[0]


# fetch n D : contents of the first cell named n, else DEFAULT.  Z + EQ + Church bool.
fetch = lambda n: L.Z(lambda rec: lambda d:
    L.ISNIL(d)
        (lambda _u: DEFAULT)
        (lambda _u: (lambda c:
            L.EQ(numeral_of(L.FST(c)))(n)
                (lambda _v: L.SND(c))
                (lambda _v: rec(L.TAIL(d)))
                (L.I))(L.HEAD(d)))
        (L.I))
