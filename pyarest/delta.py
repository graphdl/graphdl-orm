"""The delta fast-path (spec §4.2 / D5, amendment §3.1.2): a native tagged representation the
runtime evaluates for speed. The lambda kernel remains the ground truth that establishes
mu = lfp tau; this path is held observationally equal to it BY THE DIFFERENTIAL ORACLE
(tests/test_oracle.py — seeded metamorphic sweep over the base, the ⊥ edges, theta1 and the
constraint algebra; plus test_bottom/test_boundary regression pins). Nothing in the
definitions references this path (the FFP objects are the same sequences of atoms). Any new
primitive must land in BOTH paths and in the oracle's leaf set.

Native object: a scalar IS an atom (its ORM-typed value), a Python tuple IS a sequence, BOT_D
is bottom (a non-sequence singleton — it can never be indexed or iterated as data), APP_D
heads an application node ⟨APP, f, x⟩ and is SHARED with the λ kernel (lam.APPTAG) so App
nodes survive Scott↔native conversion. Every primitive is a native tuple/scalar op guarded to
⊥ outside its stated shape; the store is a dict (O(1) fetch) merging the native base, the
compiled defs, and BRIDGED registered defs (the enumerable boundary works on both paths);
the step's DEFS-in-D binding resolves first. Metacomposition is the only mechanism, as in
the ground truth.
"""
from . import lam as L
from . import defs as _defs_mod

class _Bot:
    """⊥ as a non-sequence singleton: it can never be indexed, iterated, or mistaken for
    data (the audit's A1 — a tuple sentinel leaked through _isseq into ALPHA as a list)."""
    __slots__ = ()
    def __repr__(self):
        return "⊥"


BOT_D = _Bot()                                               # bottom sentinel (not a tuple!)
APP_D = L.APPTAG                                             # application-node head — SHARED with
                                                             # the λ kernel so App nodes cross paths
_T, _F = "T", "F"
_isseq = lambda o: type(o) is tuple


def _mkseq(items):
    """⊥-collapsing sequence construction (Backus §11.2.1): ⟨…,⊥,…⟩ IS ⊥."""
    t = tuple(items)
    return BOT_D if any(i is BOT_D for i in t) else t


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
    if n is L.APPTAG:                                        # the app sentinel is an ATOM value,
        return L.ATOM(n)                                     # never a data sequence
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
# operand-shape guards (§11.2.3): a primitive outside its stated shape is ⊥
_pair = lambda o: _isseq(o) and len(o) == 2                  # ⟨x, y⟩
_pair_xs = lambda o: _pair(o) and _isseq(o[1])               # ⟨x, ⟨…⟩⟩
_pair_sx = lambda o: _pair(o) and _isseq(o[0])               # ⟨⟨…⟩, x⟩
_pair_ss = lambda o: _pair(o) and _isseq(o[0]) and _isseq(o[1])

def _binop(f):
    return lambda mu, o: f(o[0], o[1]) if (_pair(o) and _num(o[0], o[1])) else BOT_D

def _cmp(rel):
    def g(mu, o):
        if not _pair(o):
            return BOT_D
        a, b = o[0], o[1]
        ok = not _isseq(a) and not _isseq(b) and (_num(a, b) or type(a) is type(b))
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
    return _mkseq(mu((APP_D, f, x)) for f in whole[1:])

def _const(mu, o):
    return o[0][1] if len(o[0]) >= 2 else BOT_D              # ⟨CONST⟩ with no payload is ⊥

def _cond(mu, o):
    whole, x = o[0], o[1]
    if len(whole) < 4:
        return BOT_D
    pv = mu((APP_D, whole[1], x))
    return mu((APP_D, whole[2], x)) if pv == _T else (mu((APP_D, whole[3], x)) if pv == _F else BOT_D)

def _alpha(mu, o):
    whole, x = o[0], o[1]
    if len(whole) < 2:
        return BOT_D
    return () if x == () else (_mkseq(mu((APP_D, whole[1], xi)) for xi in x) if _isseq(x) else BOT_D)

def _insert(mu, o):
    # /f as an iterative right fold: the same semantics as the recursive definition (each
    # application reduced by mu, pairs ⊥-collapsing), without one host frame per element,
    # so ARC-scale populations do not exhaust the stack on the runtime path
    whole, x = o[0], o[1]
    if len(whole) < 2 or not _isseq(x) or len(x) == 0:
        return BOT_D
    acc = x[-1]
    for xi in reversed(x[:-1]):
        acc = mu((APP_D, whole[1], _mkseq((xi, acc))))
    return acc

def _while(mu, o):
    # (while p f) as a native loop: Backus's definition is tail recursion, iterated here
    whole, x = o[0], o[1]
    if len(whole) < 3:
        return BOT_D
    while True:
        pv = mu((APP_D, whole[1], x))
        if pv == _F:
            return x
        if pv != _T:
            return BOT_D
        x = mu((APP_D, whole[2], x))

def _bu(mu, o):
    whole, y = o[0], o[1]
    return mu((APP_D, whole[1], (whole[2], y))) if len(whole) >= 3 else BOT_D

def _trans(mu, o):
    if not _isseq(o) or any(not _isseq(r) for r in o):
        return BOT_D
    if len(o) == 0:
        return ()
    n = len(o[0])
    if any(len(r) != n for r in o):
        return BOT_D
    return tuple(tuple(r[i] for r in o) for i in range(n))


_NATIVE = {
    "tl": lambda mu, o: o[1:] if (_isseq(o) and len(o) >= 1) else BOT_D,
    "id": lambda mu, o: o,
    "atom": lambda mu, o: BOT_D if o is BOT_D else (_F if (_isseq(o) and len(o) > 0) else _T),
    "null": lambda mu, o: BOT_D if o is BOT_D else (_T if o == () else _F),
    "eq": lambda mu, o: (_T if _eqobj(o[0], o[1]) else _F) if _pair(o) else BOT_D,
    "apndl": lambda mu, o: (o[0],) + o[1] if _pair_xs(o) else BOT_D,
    "apndr": lambda mu, o: o[0] + (o[1],) if _pair_sx(o) else BOT_D,
    "distl": lambda mu, o: tuple((o[0], y) for y in o[1]) if _pair_xs(o) else BOT_D,
    "distr": lambda mu, o: tuple((x, o[1]) for x in o[0]) if _pair_sx(o) else BOT_D,
    "length": lambda mu, o: len(o) if _isseq(o) else BOT_D,
    "reverse": lambda mu, o: tuple(reversed(o)) if _isseq(o) else BOT_D,
    "cat": lambda mu, o: o[0] + o[1] if _pair_ss(o) else BOT_D,
    "not": lambda mu, o: _F if o == _T else (_T if o == _F else BOT_D),
    "and": lambda mu, o: ((_T if (o[0] == _T and o[1] == _T) else _F)
        if (o[0] in (_T, _F) and o[1] in (_T, _F)) else BOT_D) if _pair(o) else BOT_D,
    "or": lambda mu, o: ((_T if (o[0] == _T or o[1] == _T) else _F)
        if (o[0] in (_T, _F) and o[1] in (_T, _F)) else BOT_D) if _pair(o) else BOT_D,
    "1r": lambda mu, o: o[-1] if (_isseq(o) and len(o) >= 1) else BOT_D,
    "tlr": lambda mu, o: o[:-1] if (_isseq(o) and len(o) >= 1) else BOT_D,
    "trans": _trans,
    "rotl": lambda mu, o: o[1:] + o[:1] if _isseq(o) else BOT_D,
    "rotr": lambda mu, o: o[-1:] + o[:-1] if _isseq(o) else BOT_D,
    "+": _binop(lambda a, b: a + b), "-": _binop(lambda a, b: a - b), "*": _binop(lambda a, b: a * b),
    "div": lambda mu, o: o[0] / o[1] if (_pair(o) and _num(o[0], o[1]) and o[1] != 0) else BOT_D,
    "ge": _cmp(lambda a, b: a >= b), "gt": _cmp(lambda a, b: a > b),
    "le": _cmp(lambda a, b: a <= b), "lt": _cmp(lambda a, b: a < b),
    "apply": lambda mu, o: mu((APP_D, o[0], o[1])) if _pair(o) else BOT_D,
    "COMP": _comp, "CONS": _cons, "CONST": _const, "COND": _cond,
    "ALPHA": _alpha, "INSERT": _insert, "WHILE": _while, "BU": _bu,
}
for _i in range(1, 33):
    _NATIVE[_i] = _sel(_i)


# ============================ the native store + reducer ======================
_cache = {"version": -1, "defs": {}}


def _bridge(fn):
    """Adapt a registered Scott-flavored impl(mu)(operand) — the enumerable boundary:
    render/httpFetch/upsert and any late-registered host def — to the native path. Operand
    and result cross by conversion; the mu handed to the impl speaks Scott. App nodes
    survive both directions because the head sentinel is shared (L.APPTAG)."""
    def native_impl(mu, x):
        scott_mu = lambda e: native_to_scott(mu(scott_to_native(e)))
        return scott_to_native(fn(scott_mu)(native_to_scott(x)))
    return native_impl


def _store():
    if _cache["version"] != _defs_mod.version:
        d = {name: (0, fn) for name, fn in _NATIVE.items()}          # 0 = native primitive
        for name, (kind, impl) in _defs_mod.latest.items():          # recency across kinds, so the
            if kind == "compiled":                                   # native store resolves a name
                d[name] = (1, scott_to_native(impl))                 # exactly like the Scott ↑
            elif name not in _NATIVE:                                # registered: native twin if we
                d[name] = (0, _bridge(impl))                         # have one, else bridge the impl
        _cache["version"], _cache["defs"] = _defs_mod.version, d
    return _cache["defs"]


def _make_mu(store, step_defs):
    def mu(e):
        # a value is its own meaning; an application node ⟨APP, f, x⟩ reduces (metacomposition)
        if type(e) is tuple and len(e) == 3 and e[0] is APP_D:
            f = mu(e[1]); x = mu(e[2])                               # reduce operator, then operand (cbv)
            if f is BOT_D or x is BOT_D:                             # §13.3.1: ρ⊥ = ⊥ and every
                return BOT_D                                         # function is ⊥-preserving
            if _isseq(f):                                            # seq operator -> metacomposition
                return mu((APP_D, f[0], (f, x))) if f else BOT_D     # (φ as operator is ⊥)
            sd = step_defs.get(f)
            if sd is not None:                                       # the step's DEFS cell first
                return mu((APP_D, sd, x))                            # (Def. AREST / Cor. closure)
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
    mu = _make_mu(_store(), _defs_mod.step_native(scott_to_native))
    return native_to_scott(mu((APP_D, scott_to_native(f), scott_to_native(x))))


def meaning(e):
    """mu e on the fast path — reduce an FFP expression to its normal form (Scott object out)."""
    mu = _make_mu(_store(), _defs_mod.step_native(scott_to_native))
    return native_to_scott(mu(scott_to_native(e)))
