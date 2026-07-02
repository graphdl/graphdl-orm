"""The delta fast-path (spec §4.2 / D5): a native tagged representation the runtime evaluates
for speed, OBSERVATIONALLY EQUAL to the lambda/Scott encoding in lam.py + reduce.py. The lambda
kernel remains the ground truth that establishes mu = lfp tau; nothing in the definitions
references this path (the FFP objects are the same sequences of atoms). It exists only to remove
the per-operation closure overhead and the O(n) store walk.

Native object: a scalar IS an atom (its ORM-typed value), a Python tuple IS a sequence, BOT_D is
bottom, APP_D heads an application node ⟨APP, f, x⟩. Every primitive is a native tuple/scalar op;
the store is a dict (O(1) fetch); metacomposition is the only mechanism, as in the ground truth.
"""
from . import lam as L
from . import defs as _defs_mod

BOT_D = ("#bot#",)                                            # bottom sentinel
APP_D = ("#app#",)                                           # application-node head sentinel
_T, _F = "T", "F"
_isseq = lambda o: type(o) is tuple


# ============================ native <-> Scott conversion =====================
def scott_to_native(o):
    box = []
    def on_atom(v):
        box.append(v)
    def on_seq(l):
        items, cur = [], l
        while not L._is_nil(cur):
            items.append(scott_to_native(L.HEAD(cur))); cur = L.TAIL(cur)
        box.append(tuple(items))
    marker = o(on_atom)(on_seq)(BOT_D)
    return box[0] if box else marker


def native_to_scott(n):
    if n is BOT_D:
        return L.BOT
    if _isseq(n):
        l = L.NIL
        for e in reversed(n):
            l = L.CONS(native_to_scott(e))(l)
        return L.SEQ(l)
    return L.ATOM(n)


# ============================ native primitives ===============================
def _sel(i):
    return lambda mu, o: o[i - 1] if (_isseq(o) and len(o) >= i) else BOT_D

def _eqobj(a, b):
    if _isseq(a) and _isseq(b):
        return len(a) == len(b) and all(_eqobj(x, y) for x, y in zip(a, b))
    if _isseq(a) or _isseq(b):
        return False
    return type(a) is type(b) and a == b                     # NATEQ on atoms (same ORM type + equal)

_num = lambda a, b: isinstance(a, (int, float)) and isinstance(b, (int, float)) \
    and not isinstance(a, bool) and not isinstance(b, bool)

def _binop(f):
    return lambda mu, o: f(o[0], o[1]) if _num(o[0], o[1]) else BOT_D

def _cmp(rel):
    def g(mu, o):
        a, b = o[0], o[1]
        ok = a is not BOT_D and b is not BOT_D and (_num(a, b) or type(a) is type(b))
        return (_T if rel(a, b) else _F) if ok else BOT_D
    return g

# controlling operators receive (mu, arg) where arg = ⟨⟨OP, params⟩, x⟩; they build the next
# expression (an App node) or the value, which the reducer then reduces via mu.
def _comp(mu, o):
    whole, x = o[0], o[1]
    acc = x
    for f in reversed(whole[1:]):
        acc = (APP_D, f, acc)
    return acc

def _cons(mu, o):
    whole, x = o[0], o[1]
    return tuple(mu((APP_D, f, x)) for f in whole[1:])

def _const(mu, o):
    return o[0][1] if o[1] is not BOT_D else BOT_D

def _cond(mu, o):
    whole, x = o[0], o[1]
    pv = mu((APP_D, whole[1], x))
    return mu((APP_D, whole[2], x)) if pv == _T else (mu((APP_D, whole[3], x)) if pv == _F else BOT_D)

def _alpha(mu, o):
    whole, x = o[0], o[1]
    return () if x == () else (tuple(mu((APP_D, whole[1], xi)) for xi in x) if _isseq(x) else BOT_D)

def _insert(mu, o):
    whole, x = o[0], o[1]
    if not _isseq(x) or len(x) == 0:
        return BOT_D
    return x[0] if len(x) == 1 else mu((APP_D, whole[1], (x[0], mu((APP_D, whole, x[1:])))))

def _while(mu, o):
    whole, x = o[0], o[1]
    pv = mu((APP_D, whole[1], x))
    return mu((APP_D, whole, mu((APP_D, whole[2], x)))) if pv == _T else (x if pv == _F else BOT_D)


_NATIVE = {
    "tl": lambda mu, o: o[1:] if (_isseq(o) and len(o) >= 1) else BOT_D,
    "id": lambda mu, o: o,
    "atom": lambda mu, o: BOT_D if o is BOT_D else (_F if (_isseq(o) and len(o) > 0) else _T),
    "null": lambda mu, o: BOT_D if o is BOT_D else (_T if o == () else _F),
    "eq": lambda mu, o: (_T if _eqobj(o[0], o[1]) else _F) if (_isseq(o) and len(o) == 2) else BOT_D,
    "apndl": lambda mu, o: (o[0],) + o[1],
    "apndr": lambda mu, o: o[0] + (o[1],),
    "distl": lambda mu, o: tuple((o[0], y) for y in o[1]),
    "distr": lambda mu, o: tuple((x, o[1]) for x in o[0]),
    "length": lambda mu, o: len(o) if _isseq(o) else BOT_D,
    "reverse": lambda mu, o: tuple(reversed(o)) if _isseq(o) else BOT_D,
    "cat": lambda mu, o: o[0] + o[1],
    "not": lambda mu, o: _F if o == _T else (_T if o == _F else BOT_D),
    "and": lambda mu, o: _T if (o[0] == _T and o[1] == _T) else _F,
    "or": lambda mu, o: _T if (o[0] == _T or o[1] == _T) else _F,
    "1r": lambda mu, o: o[-1] if (_isseq(o) and len(o) >= 1) else BOT_D,
    "tlr": lambda mu, o: o[:-1] if (_isseq(o) and len(o) >= 1) else BOT_D,
    "+": _binop(lambda a, b: a + b), "-": _binop(lambda a, b: a - b), "*": _binop(lambda a, b: a * b),
    "ge": _cmp(lambda a, b: a >= b), "gt": _cmp(lambda a, b: a > b),
    "le": _cmp(lambda a, b: a <= b), "lt": _cmp(lambda a, b: a < b),
    "apply": lambda mu, o: mu((APP_D, o[0], o[1])),
    "COMP": _comp, "CONS": _cons, "CONST": _const, "COND": _cond,
    "ALPHA": _alpha, "INSERT": _insert, "WHILE": _while,
}
for _i in range(1, 33):
    _NATIVE[_i] = _sel(_i)


# ============================ the native store + reducer ======================
_cache = {"version": -1, "defs": {}}


def _store():
    if _cache["version"] != _defs_mod.version:
        d = {name: (0, fn) for name, fn in _NATIVE.items()}          # 0 = native primitive
        for name, obj in _defs_mod.compiled.items():
            d[name] = (1, scott_to_native(obj))                      # 1 = compiled FFP object (native)
        _cache["version"], _cache["defs"] = _defs_mod.version, d
    return _cache["defs"]


def _make_mu(store):
    def mu(e):
        # a value is its own meaning; an application node ⟨APP, f, x⟩ reduces (metacomposition)
        if type(e) is tuple and len(e) == 3 and e[0] is APP_D:
            f = mu(e[1]); x = mu(e[2])                               # reduce operator, then operand (cbv)
            if _isseq(f):                                            # seq operator -> metacomposition
                return mu((APP_D, f[0], (f, x)))
            hit = store.get(f)
            if hit is None:
                return BOT_D
            kind, impl = hit
            return mu(impl(mu, x)) if kind == 0 else mu((APP_D, impl, x))
        return e
    return mu


def apply(f, x):
    """The fast-path FFP application: convert operands to native, reduce, convert the result back
    to a Scott object (so callers/from_lam are unchanged). Observationally equal to reduce.apply."""
    mu = _make_mu(_store())
    return native_to_scott(mu((APP_D, scott_to_native(f), scott_to_native(x))))


def meaning(e):
    """mu e on the fast path — reduce an FFP expression to its normal form (Scott object out)."""
    mu = _make_mu(_store())
    return native_to_scott(mu(scott_to_native(e)))
