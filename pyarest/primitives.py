"""Backus FP primitives (§11.2.3): selectors, tl, id, atom, eq, null."""
from .objects import Atom, Seq, seq, BOTTOM, T, F, PHI, is_atom, is_seq
from .defs import DEFS


def _selector(n):
    def sel(x):
        if is_seq(x) and len(x.items) >= n:
            return x.items[n - 1]
        return BOTTOM
    return sel


def _tl(x):
    if x == PHI:
        return BOTTOM
    if is_seq(x) and len(x.items) >= 1:
        return Seq(x.items[1:]) if len(x.items) >= 2 else PHI
    return BOTTOM


def _id(x):
    return x


def _atom(x):
    if x is BOTTOM:
        return BOTTOM
    return T if is_atom(x) else F


def _eq(x):
    if is_seq(x) and len(x.items) == 2:
        return T if x.items[0] == x.items[1] else F
    return BOTTOM


def _null(x):
    if x is BOTTOM:
        return BOTTOM
    return T if x == PHI else F


def register_primitives(defs=DEFS):
    for n in range(1, 33):
        defs.register(n, _selector(n))
    defs.register("tl", _tl)
    defs.register("id", _id)
    defs.register("atom", _atom)
    defs.register("eq", _eq)
    defs.register("null", _null)


register_primitives()
