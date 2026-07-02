"""Objects (Backus §11.2.1) and expressions (§13.2) — the substrate the paper
takes as given. An object is an atom, a sequence ⟨x1..xn⟩, or ⊥; an expression
adds an application (f:x). Atoms are strings or numbers, exactly as Backus.
"""


class Atom:
    """An atom (§11.2.1). Optionally carries its raw data type `t` for storage —
    a portable data-type name like 'Unsigned Integer' or 'String', not the ORM
    value type (which is schema-level; a value type *has* a data type). A value
    may serve as an id, but what it stores is the data type. Substrate atoms
    (function symbols, T/F/φ/#) leave it None. Equality is by value and data type."""
    __slots__ = ("v", "t")

    def __init__(self, v, t=None):
        self.v = v
        self.t = t

    def __eq__(self, o):
        return (isinstance(o, Atom) and o.t == self.t
                and type(o.v) is type(self.v) and o.v == self.v)

    def __hash__(self):
        return hash(("A", type(self.v).__name__, self.v, self.t))

    def __repr__(self):
        return str(self.v) if self.t is None else f"{self.v}·{self.t}"


class Seq:
    __slots__ = ("xs",)

    def __init__(self, xs):
        self.xs = tuple(xs)

    def __eq__(self, o):
        return isinstance(o, Seq) and o.xs == self.xs

    def __hash__(self):
        return hash(("S", self.xs))

    def __repr__(self):
        return "⟨" + ",".join(map(repr, self.xs)) + "⟩"


class App:
    """An application expression (f:x), Backus §13.2 — reduced away by μ."""
    __slots__ = ("f", "x")

    def __init__(self, f, x):
        self.f = f
        self.x = x

    def __repr__(self):
        return f"({self.f!r}:{self.x!r})"


class _Bottom:
    def __repr__(self):
        return "⊥"


BOT = _Bottom()
PHI = Seq(())                    # the empty sequence φ


def sq(*xs):
    """⊥-preserving sequence constructor (§11.2.1)."""
    return BOT if any(map(lambda x: x is BOT, xs)) else Seq(xs)
