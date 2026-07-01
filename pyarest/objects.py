"""The object domain O (Backus §11.2.1): atom | sequence | ⊥."""
from __future__ import annotations


class Bottom:
    """⊥ — bottom / undefined. Unique singleton."""
    _inst = None

    def __new__(cls):
        if cls._inst is None:
            cls._inst = super().__new__(cls)
        return cls._inst

    def __repr__(self):
        return "⊥"


BOTTOM = Bottom()


class Atom:
    """An atom: a symbol (str) or a number (int/float)."""
    __slots__ = ("value",)

    def __init__(self, value):
        self.value = value

    def __eq__(self, other):
        return (
            isinstance(other, Atom)
            and type(other.value) is type(self.value)
            and other.value == self.value
        )

    def __hash__(self):
        return hash(("A", type(self.value).__name__, self.value))

    def __repr__(self):
        return str(self.value)


class Seq:
    """A sequence ⟨x1,...,xn⟩ of objects, held as an immutable tuple."""
    __slots__ = ("items",)

    def __init__(self, items):
        self.items = tuple(items)

    def __eq__(self, other):
        return isinstance(other, Seq) and other.items == self.items

    def __hash__(self):
        return hash(("S", self.items))

    def __repr__(self):
        return "⟨" + ", ".join(map(repr, self.items)) + "⟩"


PHI = Seq(())          # the empty sequence — both atom and sequence
T = Atom("T")
F = Atom("F")
DEFAULT = Atom("#")    # fetch default (Backus §13.3.4)


def seq(*items):
    """⊥-preserving sequence constructor (Backus §11.2.1)."""
    for x in items:
        if x is BOTTOM:
            return BOTTOM
    return Seq(items)


def is_bottom(x):
    return x is BOTTOM


def is_seq(x):
    return isinstance(x, Seq)


def is_atom(x):
    return isinstance(x, Atom) or x == PHI   # φ is both
