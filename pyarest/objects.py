"""Objects as self-dispatching lambdas (Scott encoding); atom identity is a
Church numeral. Branching is application — an object takes one handler per case
and applies its own. No `if`, no `isinstance`.

    object = λ on_atom. λ on_seq. λ on_app. λ on_bottom. λ on_prim. <selects>

  atom : carries a Church numeral (its identity — the characteristica
         universalis, the I Ching's binary: an atom *is* a number)
  seq  : carries a Church list of objects (walking it is a fold)
  app  : carries operator and operand — an application (o:x), reduced by μ
  ⊥    : selects on_bottom
  prim : wraps a host primitive λ, so primitives live in DEFS like everything else
"""
from functools import reduce as _fold
from . import lam as L

atom = lambda a: lambda oa: lambda os: lambda op: lambda ob: lambda opr: oa(a)
sq = lambda s: lambda oa: lambda os: lambda op: lambda ob: lambda opr: os(s)
app = lambda o: lambda x: lambda oa: lambda os: lambda op: lambda ob: lambda opr: op(o)(x)
bot = lambda oa: lambda os: lambda op: lambda ob: lambda opr: ob
prim = lambda g: lambda oa: lambda os: lambda op: lambda ob: lambda opr: opr(g)

NIL = L.NIL

# --- the reader: intern surface names to numeral-tagged atoms (host boundary) ---
_id = {}
_nm = {}
_nat = lambda i: _fold(lambda n, _k: L.SUCC(n), range(i), L.ZERO)


def sym(name):
    """Intern a surface name to a numeral-identified atom (reader boundary)."""
    i = _id.setdefault(name, len(_id))
    _nm[i] = name
    return atom(_nat(i))


# sequence objects — folds, not loops
seq = lambda *xs: sq(_fold(lambda t, h: L.CONS(h)(t), reversed(xs), L.NIL))
seq2 = lambda a: lambda b: sq(L.CONS(a)(L.CONS(b)(L.NIL)))
PHI = sq(L.NIL)

# extract an atom's numeral identity by dispatch (non-atoms → 0)
numeral_of = lambda o: o(lambda p: p)(lambda s: L.ZERO)(lambda f: lambda x: L.ZERO)(L.ZERO)(lambda g: L.ZERO)

DEFAULT = sym("#")                       # fetch's "not found" (Backus §13.3.4)
_HASH = numeral_of(DEFAULT)
# is this object the atom # ?  (Church boolean, by dispatch — only atoms can be)
is_default = lambda o: o(lambda p: L.EQ(p)(_HASH))(lambda s: L.FALSE)(lambda f: lambda x: L.FALSE)(L.FALSE)(lambda g: L.FALSE)

# --- reify: object → host data for tests (recursion by Z, branch by dispatch) ---
_toi = lambda n: n(lambda k: k + 1)(0)
reify = L.Z(lambda self: lambda o: o
            (lambda a: _nm[_toi(a)])                                   # atom → name
            (lambda s: s(lambda h: lambda acc: (self(h),) + acc)(()))  # seq  → tuple
            (lambda f: lambda x: ("app", self(f), self(x)))            # app
            ("⊥")                                                      # ⊥
            (lambda g: "prim"))                                        # prim
