"""The Python kernel in ONE file, shaped like the other hosts (Samuel,
2026-07-04: rust is one source file, csharp four, java five; a host is a
reducer plus a wrap). Five strata merged, in dependency order, each keeping
its own docstring below: lam (the lambda substrate), defs (the definition
store and enumerable boundary), delta (the native fast path), reduce (mu, the
ground truth), prims (the Backus base). Sibling references resolve through
_KERNEL, the module's self-alias; the package init aliases the old module
names (pyarest.lam and kin) to this module, so every existing import keeps
working unchanged."""
import sys as _sys

_KERNEL = _sys.modules[__name__]
L = _KERNEL

# ===================== lam: the lambda substrate =====================
"""The lambda-calculus substrate (Backus §12.8-12.9, his own FP-on-lambda).

Raw lambda is used for the STRUCTURE and the MACHINERY only: booleans, Church pairs, the
Scott-encoded object union ATOM | SEQ | BOT, Scott lists, Backus's primitives and folds,
and the Y combinator (Backus's lim f_i / least fixed point). Recursion is always Y, never
Python's call stack.

The boundary for VALUES is the ORM value-type object layer (not Church numerals): an atom
carries a native, ORM-typed value, and the only operation on values is equality by ORM
value type — NATEQ, a boundary op (same type and equal). Everything above a value is
lambda; the value itself is the leaf. The reader/printer is the tokenizer boundary.
"""
# ============================ pure lambda combinators =========================
I = lambda x: x
K = lambda x: lambda y: x
Y = lambda f: (lambda x: f(lambda v: x(x)(v)))(lambda x: f(lambda v: x(x)(v)))  # lfp (§12.8)

# ---- Church booleans (a boolean IS its selector) ----
TRUE  = lambda a: lambda b: a
FALSE = lambda a: lambda b: b
NOT = lambda p: p(FALSE)(TRUE)
AND = lambda p: lambda q: p(q)(FALSE)
OR  = lambda p: lambda q: p(TRUE)(q)
IF  = lambda p: lambda th: lambda el: p(th)(el)()            # branches are thunks; force the chosen one

# ---- Church pairs (structural; used by the reducer/store, not for values) ----
PAIR = lambda a: lambda b: lambda s: s(a)(b)
FST  = lambda p: p(TRUE)
SND  = lambda p: p(FALSE)

# the application-node head sentinel, shared by BOTH evaluators (reduce builds it as an
# ATOM value, delta as a native tuple head) so App nodes survive Scott↔native conversion
APPTAG = ("#APP#",)

# ============================ objects, lambda-encoded =========================
# object = ATOM v | SEQ l | BOT   (Scott 3-way union; v is a NATIVE ORM-typed value).
# match: o(onAtom)(onSeq)(onBot).
ATOM = lambda v: lambda a: lambda s: lambda b: a(v)
SEQ  = lambda l: lambda a: lambda s: lambda b: s(l)
BOT  = lambda a: lambda s: lambda b: b
ATOMP = lambda o: o(lambda v: TRUE)(lambda l: LNULL(l))(BOT)  # PHI is both atom and sequence
SEQP  = lambda o: o(lambda v: FALSE)(lambda l: TRUE)(BOT)

# ---- Scott lists (sequence payloads); match: l(onNil)(onCons) ----
NIL  = lambda n: lambda c: n
CONS = lambda h: lambda t: lambda n: lambda c: c(h)(t)
HEAD = lambda l: l(BOT)(lambda h: lambda t: h)
TAIL = lambda l: l(NIL)(lambda h: lambda t: t)
LNULL = lambda l: l(TRUE)(lambda h: lambda t: FALSE)
FOLDR = Y(lambda rec: lambda f: lambda z: lambda l:           # foldr f z ⟨x1..xn⟩
        l(z)(lambda h: lambda t: f(h)(rec(f)(z)(t))))
MAPL  = Y(lambda rec: lambda f: lambda l:
        l(NIL)(lambda h: lambda t: CONS(f(h))(rec(f)(t))))
APPEND = Y(lambda rec: lambda p: lambda q:
        p(q)(lambda h: lambda t: CONS(h)(rec(t)(q))))

# ============================ the value boundary (ORM value type) =============
# NATEQ : the sole operation on values — equality by ORM value type (same type AND equal).
# a and b are native leaf values; the result is a Church boolean the machinery branches on.
NATEQ = lambda a: lambda b: TRUE if (type(a) is type(b) and a == b) else FALSE

LISTEQ = lambda eq: Y(lambda rec: lambda p: lambda q:         # structural list equality
        p(LNULL(q))
         (lambda ph: lambda pt: q(FALSE)(lambda qh: lambda qt: AND(eq(ph)(qh))(rec(pt)(qt)))))
EQOBJ = Y(lambda eq: lambda a: lambda b:                      # object equality (values via NATEQ)
        a(lambda va: b(lambda vb: NATEQ(va)(vb))(lambda lb: FALSE)(FALSE))
         (lambda la: b(lambda vb: FALSE)(lambda lb: LISTEQ(eq)(la)(lb))(FALSE))
         (FALSE))

PHI = SEQ(NIL)                                                # the empty sequence

# ---- the bottom discipline (§11.2.1): a sequence containing ⊥ IS ⊥ ----
ISBOT  = lambda o: o(lambda v: FALSE)(lambda l: FALSE)(TRUE)  # is the object ⊥?
ANYBOT = lambda l: FOLDR(lambda h: lambda a: OR(ISBOT(h))(a))(FALSE)(l)
SEQC   = lambda l: IF(ANYBOT(l))(lambda: BOT)(lambda: SEQ(l)) # ⊥-collapsing constructor

# ============================ Backus primitives (lambda) ======================
_list = lambda o: o(lambda v: NIL)(lambda l: l)(NIL)          # the Scott list inside a SEQ

def SEL(n):
    """selector n = HEAD ∘ TAIL^(n-1) — a pure HEAD/TAIL composition (no counter, no
    Church numeral); the nested term is built once here, then reduced by lambda application."""
    g = HEAD
    for _ in range(n - 1):
        g = (lambda gg: lambda l: gg(TAIL(l)))(g)
    return lambda o: o(lambda v: BOT)(lambda l: g(l))(BOT)

HD   = lambda o: o(lambda v: BOT)(lambda l: HEAD(l))(BOT)
TL   = lambda o: o(lambda v: BOT)(lambda l: l(BOT)(lambda h: lambda t: SEQ(t)))(BOT)
ID   = lambda o: o
APNDL = lambda o: SEQ(CONS(HEAD(_list(o)))(_list(HEAD(TAIL(_list(o))))))          # apndl:⟨x,⟨..⟩⟩
APNDR = lambda o: SEQ(APPEND(_list(HEAD(_list(o))))(CONS(HEAD(TAIL(_list(o))))(NIL)))  # apndr:⟨⟨..⟩,x⟩
DISTL = lambda o: (lambda x: lambda ys: SEQ(MAPL(lambda y: SEQ(CONS(x)(CONS(y)(NIL))))(ys)))(
                    HEAD(_list(o)))(_list(HEAD(TAIL(_list(o)))))                  # distl:⟨x,⟨y1..⟩⟩
DISTR = lambda o: (lambda xs: lambda y: SEQ(MAPL(lambda x: SEQ(CONS(x)(CONS(y)(NIL))))(xs)))(
                    _list(HEAD(_list(o))))(HEAD(TAIL(_list(o))))                  # distr:⟨⟨x1..⟩,y⟩

REVL  = lambda l: FOLDR(lambda h: lambda a: APPEND(a)(CONS(h)(NIL)))(NIL)(l)   # reverse a Scott list
ROTL  = lambda o: o(lambda v: BOT)(lambda l:                                   # rotl:⟨x1..xn⟩=⟨x2..xn,x1⟩
        l(SEQ(NIL))(lambda h: lambda t: SEQ(APPEND(t)(CONS(h)(NIL)))))(BOT)
ROTR  = lambda o: o(lambda v: BOT)(lambda l:                                   # rotr:⟨x1..xn⟩=⟨xn,x1..⟩
        l(SEQ(NIL))(lambda h: lambda t:
          (lambda r: SEQ(CONS(HEAD(r))(REVL(TAIL(r)))))(REVL(CONS(h)(t)))))(BOT)

_ALLB = lambda p: FOLDR(lambda h: lambda a: AND(p(h))(a))(TRUE)
_ANYB = lambda p: FOLDR(lambda h: lambda a: OR(p(h))(a))(FALSE)
_trans_rows = Y(lambda rec: lambda rl:                        # transpose a list of row-SEQs
    IF(_ANYB(lambda r: NOT(SEQP(r)))(rl))(lambda: BOT)(lambda:               # an atom row -> ⊥
    IF(LNULL(rl))(lambda: PHI)(lambda:                                        # trans:φ = φ
    IF(_ALLB(lambda r: LNULL(_list(r)))(rl))(lambda: PHI)(lambda:             # all rows spent
    IF(_ANYB(lambda r: LNULL(_list(r)))(rl))(lambda: BOT)(lambda:             # ragged -> ⊥
    (lambda rest: rest(lambda v: BOT)(lambda restl:
        SEQ(CONS(SEQ(MAPL(lambda r: HEAD(_list(r)))(rl)))(restl)))(BOT))(
        rec(MAPL(TL)(rl))))))))
TRANS = lambda o: o(lambda v: BOT)(lambda rl: _trans_rows(rl))(BOT)           # trans (§11.2.3)

_tobool = lambda b: b(True)(False)                           # Church bool -> host bool (boundary)
_is_nil = lambda l: _tobool(LNULL(l))
def _count(l):                                               # native length of a Scott list
    n = 0
    while not _is_nil(l):
        n += 1
        l = TAIL(l)
    return n
LENGTH = lambda o: o(lambda v: BOT)(lambda l: ATOM(_count(l)))(BOT)  # length : ⟨..⟩ -> a number value

# ============================ reader / printer (boundary) ======================
def atom(pyval):
    return ATOM(pyval)                                        # the value is the ORM-typed leaf, held natively
def to_lam(x):
    if isinstance(x, tuple):
        l = NIL
        for e in reversed(x):
            l = CONS(to_lam(e))(l)
        return SEQ(l)
    return ATOM(x)
def from_lam(o):
    box = []
    def on_atom(v):
        box.append(v)
    def on_seq(l):
        items, cur = [], l
        while not _is_nil(cur):
            items.append(from_lam(HEAD(cur)))
            cur = TAIL(cur)
        box.append(tuple(items))
    marker = o(on_atom)(on_seq)("⊥")
    return box[0] if box else marker


# ===================== defs: the definition store =====================
"""DEFS — the definition store (Backus §13.3.5), a lambda Scott list of cells.

A cell is ⟨key, tag, impl⟩ (a Scott list): tag TRUE = a *registered* host lambda of
the form impl(mu)(operand) — the enumerable boundary (Cor. boundary); tag FALSE = a
*compiled* FFP object o, whose meaning is mu(o : x). fetch ↑ is a lambda over the list.
register/define build the list host-side (the store is mutable state, the paper's D);
everything the reducer touches — FETCH, the cells, the keys — is lambda.
"""

_store = L.NIL            # the current Scott list of cells (newest first)
_registered = []          # names of registered (boundary) defs — host-side mirror for boundary()
fast = {}                 # the UNIVERSAL OVERRIDE INTERFACE (layering discipline): a host's
                          # optimized twin of a canonical definition, keyed by name. Resolution
                          # prefers the twin; a host lacking one degrades gracefully to the
                          # canonical form; the differential holds twin ≡ canonical.
compiled = {}             # name -> the Scott FFP object (host mirror; the delta fast-path converts these)
latest = {}               # name -> ("registered", fn) | ("compiled", obj) — recency ACROSS kinds,
                          # so the delta store resolves a name exactly like the Scott first-match
version = 0               # bumped on every register/define, so the delta store invalidates


def _cell(name, tag, impl):
    return L.CONS(name)(L.CONS(tag)(L.CONS(impl)(L.NIL)))     # key is the raw name value (ORM-typed)


def register(name, fn):
    """Register a host lambda fn = impl(mu)(operand). The boundary (Cor. boundary)."""
    global _store, version
    _store = L.CONS(_cell(name, L.TRUE, fn))(_store)
    if name not in _registered:
        _registered.append(name)
    latest[name] = ("registered", fn)
    version += 1


def override(name, fn):
    """Register a host-authored TWIN of a canonical definition (the universal override
    interface). The taxonomy: a canonical definition that is DATA (a compiled object)
    needs no entry here — it is mechanically REFLECTED into any carrier (delta's
    scott_to_native, the Rust engine's j_to_n); this registry exists for the BASE
    stratum only, whose canonical forms are lambda terms below data, so each host
    authors its native base once under the differential's equality contract. A
    registered def with NO canonical is neither reflection nor twin: swapping it is DI
    (httpFetch and kin), with no equality contract. fn is (mu, operand) -> value on the
    host's fast carrier; the canonical definition remains the meaning."""
    global version
    fast[name] = fn
    version += 1


native = {}                # name -> the delta-carrier TWIN of a compiled def (stratum 2 of the
                           # polyglot debug): byte-identical to scott_to_native of the canonical
                           # object (pinned by test_canon_native's differential), so the delta
                           # store rebuild skips the boundary for these names


def define(name, obj, native_obj=None):
    """Compile: bind `name` to an FFP object o; its meaning is mu(o : x)  (Def. reg).
    An optional native twin registers beside it for the fast path's store."""
    global _store, version
    _store = L.CONS(_cell(name, L.FALSE, obj))(_store)
    compiled[name] = obj
    latest[name] = ("compiled", obj)
    if native_obj is not None:
        native[name] = native_obj
    else:
        native.pop(name, None)                                # a redefinition EVICTS a stale twin
    version += 1


def current():
    return _store


def reset():
    global _store, _registered, compiled, latest, version
    _store, _registered, compiled, latest, version = L.NIL, [], {}, {}, version + 1
    native.clear()


# ↑key : store → ⟨found?, ⟨tag, impl⟩⟩   (first match wins, else ⟨F, _⟩); keys compared by NATEQ
FETCH = L.Y(lambda rec: lambda key: lambda d:
    d(L.PAIR(L.FALSE)(L.NIL))(lambda h: lambda t:
      L.IF(L.NATEQ(L.HEAD(h))(key))
        (lambda: L.PAIR(L.TRUE)(L.TAIL(h)))
        (lambda: rec(key)(t))))


# ============================ the step binding (Def. AREST / Cor. closure) ====
# COMPILED definitions live in a DEFS cell of the transitioned store D, so they travel with
# the state: self-modification is a store into D, and a tenant's DEFS is its own. Registered
# (host) impls cannot live in a pure object store — they remain the process registry, which
# is exactly the enumerable boundary (Def. reg). run/dispatch bind the step's D here; mu
# resolves an atom FIRST against this binding, then the process store. The binding is fixed
# for the whole reduction — Backus §14.6, the state is frozen during evaluation. The walk
# below is host machinery of the trusted base (like the mirrors above), not a definition.

_step_frame = None        # (host {name: scott_obj}, {"native": dict|None}) | None


def _aval(o):
    box = []
    o(lambda v: box.append(v))(lambda l: None)(None)
    return box[0] if box else None


def _items(l):
    out = []
    while not L._is_nil(l):
        out.append(L.HEAD(l))
        l = L.TAIL(l)
    return out


def _cells_of(D):
    """The {name: contents} view of D's cells, first match winning (Backus §13.3.5: a cell
    ⟨CELL, n, c⟩ is the definition Def n ≡ ρc; §14.3: data and function names share the
    one namespace, and usage disambiguates)."""
    d = {}
    for c in _items(L._list(D)):
        it = _items(L._list(c))
        if len(it) == 3 and _aval(it[0]) == "CELL":
            k = _aval(it[1])
            if k is not None and k not in d:                  # first match wins
                d[k] = it[2]
    return d


class step:
    """Bind D for one AST step: `with defs.step(D): …` — nestable, restored on exit.
    `fuel` bounds the step's reductions (supervision at the decidability frontier: a
    compiled step terminates by Lemma finiteness, so fuel only fires on a violated
    hypothesis or a runaway registered call; exhaustion bottoms the reduction and the
    §14.3.1 transition rule answers ⟨ERROR, unchanged D⟩)."""
    def __init__(self, D, fuel=None):
        self._frame = (_cells_of(D), {"native": None}, D, {"fuel": fuel})

    def __enter__(self):
        global _step_frame
        self._prev, _step_frame = _step_frame, self._frame
        return self

    def __exit__(self, *exc):
        global _step_frame
        _step_frame = self._prev
        return False


def step_get(key):
    """The contents of the first cell of the step's D named `key`, else None."""
    return _step_frame[0].get(key) if _step_frame is not None else None


def step_D():
    """The step's whole state, for the DEFS accessor (§14.3.3), else None."""
    return _step_frame[2] if _step_frame is not None else None


def consume_fuel():
    """False when the step's fuel is exhausted; True when unbounded or remaining."""
    if _step_frame is None:
        return True
    slot = _step_frame[3]
    if slot["fuel"] is None:
        return True
    slot["fuel"] -= 1
    return slot["fuel"] > 0


def step_native(conv):
    """The step's DEFS as native objects (converted once per binding via `conv`)."""
    if _step_frame is None:
        return {}
    if _step_frame[1]["native"] is None:
        _step_frame[1]["native"] = {k: conv(v) for k, v in _step_frame[0].items()}
    return _step_frame[1]["native"]


def boundary_population():
    """The ⟨name, origin⟩ view of the process DEFS: Def. reg's tuple less impl (a host
    callable cannot be an object) and less the unused dom/cod (open ledger item). This is
    the fact set eq. (boundary) filters."""
    return L.to_lam(tuple((n, kind) for n, (kind, _impl) in latest.items()))


def boundary():
    """Cor. boundary as the ρ-application of eq. (boundary): Filter(eq∘[s_origin,
    registered̄]) over the DEFS view, reduced by the one mu, with the names projected out
    by α(1). The informal surface of the system is a decidable fact set the algebra
    itself computes."""
    from . import canon as T
    from .reduce import apply as _ap

    def _s(*xs):
        l = L.NIL
        for x in reversed(xs):
            l = L.CONS(x)(l)
        return L.SEQ(l)

    a = L.atom
    pred = _s(a("COMP"), a("eq"), _s(a("CONS"), a(2), _s(a("CONST"), a("registered"))))
    expr = _s(a("COMP"), _s(a("ALPHA"), a(1)), T.Filter(pred))
    return list(L.from_lam(_ap(expr, boundary_population())))


# §14.3.3: "Our FFP subsystem is required to have one new primitive function, defs, named
# DEFS such that for any object x ≠ ⊥, defs:x = ρDEFS:x = D" — program access to the whole
# state, including "the essential [purpose] of computing the successor state". Outside a
# step there is no state, so the accessor is ⊥ there.
register("DEFS", lambda mu: lambda o: step_D() if _step_frame is not None else L.BOT)


# ===================== delta: the native fast path =====================
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
# Conversion is memoized on NODE IDENTITY: Scott values are immutable closures and
# commits share structure (Store/apndl reuse the untouched cells), so an unchanged
# cell converts once per process instead of once per apply. Without this the
# boundary is quadratic in |D| across a compile (the profile showed 29M calls for
# a 240-statement model). The keep-list pins ids; the cache clears when it grows
# past the bound.
_S2N_MEMO = {}
_S2N_KEEP = []


def scott_to_native(o):
    key = id(o)
    hit = _S2N_MEMO.get(key)
    if hit is not None and hit[0] is o:
        return hit[1]
    box = []
    def on_atom(v):
        box.append(v)
    def on_seq(l):
        items, cur = [], l
        while not L._is_nil(cur):
            items.append(scott_to_native(L.HEAD(cur))); cur = L.TAIL(cur)
        box.append(tuple(items))
    marker = o(on_atom)(on_seq)(BOT_D)
    res = box[0] if box else marker
    if len(_S2N_MEMO) > 500000:
        _S2N_MEMO.clear()
        _S2N_KEEP.clear()
    _S2N_MEMO[key] = (o, res)
    _S2N_KEEP.append(o)
    return res


# The way back is VALUE-INTERNED: equal native subtrees answer the identical Scott
# closure, so a result D whose cells mostly match the input D reuses their closures,
# which keeps the forward memo's identity sharing alive across applies. Without
# this, every apply rebuilds every closure and the boundary is quadratic across a
# compile regardless of the forward memo.
_N2S_MEMO = {}


def native_to_scott(n):
    if n is BOT_D:
        return L.BOT
    if n is L.APPTAG:                                        # the app sentinel is an ATOM value,
        return L.ATOM(n)                                     # never a data sequence
    if _isseq(n):
        hit = _N2S_MEMO.get(n)
        if hit is not None:
            return hit
        l = L.NIL
        for e in reversed(n):
            l = L.CONS(native_to_scott(e))(l)
        out = L.SEQ(l)
        if len(_N2S_MEMO) > 500000:
            _N2S_MEMO.clear()
        _N2S_MEMO[n] = out
        return out
    return L.ATOM(n)


# ============================ native primitives ===============================
def _d_sel(i):
    return lambda mu, o: o[i - 1] if (_isseq(o) and len(o) >= i) else BOT_D

def _eqobj(a, b):
    if _isseq(a) and _isseq(b):
        return len(a) == len(b) and all(_eqobj(x, y) for x, y in zip(a, b))
    if _isseq(a) or _isseq(b):
        return False
    return type(a) is type(b) and a == b                     # NATEQ on atoms (same ORM type + equal)

_num = lambda a, b: isinstance(a, (int, float)) and isinstance(b, (int, float)) \
    and not isinstance(a, bool) and not isinstance(b, bool)


def _d_tonum(x):
    """Arithmetic coercion: the store carries LEXICAL atoms ('120' — ORM values
    are conceptually typed, lexically stored), so a numeric-looking string is a
    number to + and kin. Anything else answers None (the caller bottoms).
    Mirrored in prims._d_tonum — the oracle law: both paths, identically."""
    if isinstance(x, bool):
        return None
    if isinstance(x, (int, float)):
        return x
    if isinstance(x, str):
        try:
            return int(x)
        except ValueError:
            try:
                return float(x)
            except ValueError:
                return None
    return None
# operand-shape guards (§11.2.3): a primitive outside its stated shape is ⊥
_pair = lambda o: _isseq(o) and len(o) == 2                  # ⟨x, y⟩
_d_pair_xs = lambda o: _pair(o) and _isseq(o[1])               # ⟨x, ⟨…⟩⟩
_d_pair_sx = lambda o: _pair(o) and _isseq(o[0])               # ⟨⟨…⟩, x⟩
_d_pair_ss = lambda o: _pair(o) and _isseq(o[0]) and _isseq(o[1])

def _binop(f):
    def g(mu, o):
        if not _pair(o):
            return BOT_D
        a, b = _d_tonum(o[0]), _d_tonum(o[1])
        return f(a, b) if a is not None and b is not None else BOT_D
    return g

def _d_cmp(rel):
    def g(mu, o):
        if not _pair(o):
            return BOT_D
        a, b = o[0], o[1]
        if _isseq(a) or _isseq(b):
            return BOT_D
        na, nb = _d_tonum(a), _d_tonum(b)
        if na is not None and nb is not None:
            # comparison coerces like arithmetic: the store's lexical numbers
            # ('305' beside a sum's 4997) order numerically wherever both
            # sides parse — the old kernel's atoms were typed, so its folds
            # compared numbers as numbers (the claude analytics family)
            return _T if rel(na, nb) else _F
        ok = _num(a, b) or type(a) is type(b)
        return (_T if rel(a, b) else _F) if ok else BOT_D
    return g

# controlling operators receive (mu, arg) where arg = ⟨⟨OP, params⟩, x⟩; they build the next
# expression (an App node) or the value, which the reducer then reduces via mu.
def _d_comp(mu, o):
    whole, x = o[0], o[1]
    acc = x
    for f in reversed(whole[1:]):
        acc = (APP_D, f, acc)
    return acc

def _d_cons(mu, o):
    whole, x = o[0], o[1]
    return _mkseq(mu((APP_D, f, x)) for f in whole[1:])

def _d_const(mu, o):
    return o[0][1] if len(o[0]) >= 2 else BOT_D              # ⟨CONST⟩ with no payload is ⊥

def _d_cond(mu, o):
    whole, x = o[0], o[1]
    if len(whole) < 4:
        return BOT_D
    pv = mu((APP_D, whole[1], x))
    return mu((APP_D, whole[2], x)) if pv == _T else (mu((APP_D, whole[3], x)) if pv == _F else BOT_D)

# ---- the functional-form override registry (Register/Resolve) ----
# The MonoCross MXContainer pattern (Samuel, 2026-07-08: "Read the
# Register/Resolve methods that I wrote into MonoCross"): an ABSTRACT
# key — the functional form's name — plus an optional NAMED INSTANCE
# maps to a native implementation, and resolution falls back in layers:
# the named instance first, the default instance next, the structural
# evaluation last (exactly MXContainer.Resolve repackaging an unknown
# name and constructing unregistered types anyway). The evaluator's
# form branches consult resolve_form; registering is open to hosts and
# targets (a GPU alpha, a distributed alpha) without touching the
# kernel.
_FORM_OVERRIDES = {}


def register_form(form, impl, name=None):
    """Register a native implementation for a functional form. impl is
    (mu, o) -> value over the same o the structural branch receives."""
    _FORM_OVERRIDES[(form, name)] = impl


def resolve_form(form, name=None):
    """The layered lookup: named instance, then the default instance,
    then None — the caller's structural path is the final fallback."""
    if name is not None and (form, name) in _FORM_OVERRIDES:
        return _FORM_OVERRIDES[(form, name)]
    return _FORM_OVERRIDES.get((form, None))


_ALPHA_POOL = None


def _alpha_free_threaded():
    import sys
    return getattr(sys, "_is_gil_enabled", lambda: True)() is False


_ALPHA_PARALLEL = _alpha_free_threaded()


def _alpha_map(mu, f, xs):
    """Backus's apply-to-all: the items are independent pure reductions
    over immutable terms — alpha's whole pitch in the FP paper — so a
    free-threaded host maps them across a thread pool (order preserved;
    pool sized by the runtime). Under the GIL the sequential map IS the
    performant form — threads only add overhead to CPU-bound reduction —
    so the parallel path gates on sys._is_gil_enabled() being False
    (python 3.13+ free-threaded builds) and engages by itself the day
    the interpreter allows it."""
    if _ALPHA_PARALLEL and len(xs) >= 8:
        global _ALPHA_POOL
        if _ALPHA_POOL is None:
            import concurrent.futures
            _ALPHA_POOL = concurrent.futures.ThreadPoolExecutor()
        return list(_ALPHA_POOL.map(lambda xi: mu((APP_D, f, xi)), xs))
    return [mu((APP_D, f, xi)) for xi in xs]


def _alpha_impl(mu, o):
    """The registered default ALPHA instance (parallel where the
    runtime allows), same o as the structural branch."""
    whole, x = o[0], o[1]
    return _mkseq(_alpha_map(mu, whole[1], x))


register_form("ALPHA", _alpha_impl)


def _d_alpha(mu, o):
    whole, x = o[0], o[1]
    if len(whole) < 2:
        return BOT_D
    if x == ():
        return ()
    if not _isseq(x):
        return BOT_D
    over = resolve_form("ALPHA")
    if over is not None:
        return over(mu, o)
    return _mkseq(_alpha_map(mu, whole[1], x))

def _d_insert(mu, o):
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

def _d_while(mu, o):
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

def _d_bu(mu, o):
    whole, y = o[0], o[1]
    return mu((APP_D, whole[1], (whole[2], y))) if len(whole) >= 3 else BOT_D

def _d_trans(mu, o):
    if not _isseq(o) or any(not _isseq(r) for r in o):
        return BOT_D
    if len(o) == 0:
        return ()
    n = len(o[0])
    if any(len(r) != n for r in o):
        return BOT_D
    return tuple(tuple(r[i] for r in o) for i in range(n))


# Python's override set: registered through the universal interface below (defs.override),
# so the fast path is a set of verified twins, not a parallel store.
_NATIVE = {
    "tl": lambda mu, o: o[1:] if (_isseq(o) and len(o) >= 1) else BOT_D,
    "id": lambda mu, o: o,
    "atom": lambda mu, o: BOT_D if o is BOT_D else (_F if (_isseq(o) and len(o) > 0) else _T),
    "null": lambda mu, o: BOT_D if o is BOT_D else (_T if o == () else _F),
    "eq": lambda mu, o: (_T if _eqobj(o[0], o[1]) else _F) if _pair(o) else BOT_D,
    "apndl": lambda mu, o: (o[0],) + o[1] if _d_pair_xs(o) else BOT_D,
    "apndr": lambda mu, o: o[0] + (o[1],) if _d_pair_sx(o) else BOT_D,
    "distl": lambda mu, o: tuple((o[0], y) for y in o[1]) if _d_pair_xs(o) else BOT_D,
    "distr": lambda mu, o: tuple((x, o[1]) for x in o[0]) if _d_pair_sx(o) else BOT_D,
    "length": lambda mu, o: len(o) if _isseq(o) else BOT_D,
    "reverse": lambda mu, o: tuple(reversed(o)) if _isseq(o) else BOT_D,
    "cat": lambda mu, o: o[0] + o[1] if _d_pair_ss(o) else BOT_D,
    "not": lambda mu, o: _F if o == _T else (_T if o == _F else BOT_D),
    "and": lambda mu, o: ((_T if (o[0] == _T and o[1] == _T) else _F)
        if (o[0] in (_T, _F) and o[1] in (_T, _F)) else BOT_D) if _pair(o) else BOT_D,
    "or": lambda mu, o: ((_T if (o[0] == _T or o[1] == _T) else _F)
        if (o[0] in (_T, _F) and o[1] in (_T, _F)) else BOT_D) if _pair(o) else BOT_D,
    "1r": lambda mu, o: o[-1] if (_isseq(o) and len(o) >= 1) else BOT_D,
    "tlr": lambda mu, o: o[:-1] if (_isseq(o) and len(o) >= 1) else BOT_D,
    "trans": _d_trans,
    "rotl": lambda mu, o: o[1:] + o[:1] if _isseq(o) else BOT_D,
    "rotr": lambda mu, o: o[-1:] + o[:-1] if _isseq(o) else BOT_D,
    "+": _binop(lambda a, b: a + b), "-": _binop(lambda a, b: a - b), "*": _binop(lambda a, b: a * b),
    "div": lambda mu, o: o[0] / o[1] if (_pair(o) and _num(o[0], o[1]) and o[1] != 0) else BOT_D,
    "ge": _d_cmp(lambda a, b: a >= b), "gt": _d_cmp(lambda a, b: a > b),
    "le": _d_cmp(lambda a, b: a <= b), "lt": _d_cmp(lambda a, b: a < b),
    "apply": lambda mu, o: mu((APP_D, o[0], o[1])) if _pair(o) else BOT_D,
    "COMP": _d_comp, "CONS": _d_cons, "CONST": _d_const, "COND": _d_cond,
    "ALPHA": _d_alpha, "INSERT": _d_insert, "WHILE": _d_while, "BU": _d_bu,
}
for _i in range(1, 33):
    _NATIVE[_i] = _d_sel(_i)


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


def _d_store():
    if _cache["version"] != _KERNEL.version:
        d = {}
        for name, (kind, impl) in _KERNEL.latest.items():          # the canonical layer first
            if kind == "compiled":
                # a native TWIN (canon.load's second vocabulary) skips the boundary;
                # () is a legitimate twin (PHI), so membership decides, not truth
                d[name] = (1, _KERNEL.native[name]
                           if name in _KERNEL.native else scott_to_native(impl))
            elif name not in _KERNEL.fast:                         # registered: bridge unless a
                d[name] = (0, _bridge(impl))                         # twin exists (degradation)
        for name, fn in _KERNEL.fast.items():                      # the override twins SHADOW the
            d[name] = (0, fn)                                        # canonical (universal interface)
        _cache["version"], _cache["defs"] = _KERNEL.version, d
    return _cache["defs"]


def _make_mu(store, step_defs):
    def mu(e):
        # a value is its own meaning; an application node ⟨APP, f, x⟩ reduces (metacomposition)
        if type(e) is tuple and len(e) == 3 and e[0] is APP_D:
            if not _KERNEL.consume_fuel():                         # supervision: exhausted
                return BOT_D                                         # fuel bottoms the step
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


def delta_apply(f, x):
    """The fast-path FFP application: convert operands to native, reduce, convert the result back
    to a Scott object (so callers/from_lam are unchanged). Observationally equal to reduce.apply."""
    mu = _make_mu(_d_store(), _KERNEL.step_native(scott_to_native))
    return native_to_scott(mu((APP_D, scott_to_native(f), scott_to_native(x))))


def delta_meaning(e):
    """mu e on the fast path — reduce an FFP expression to its normal form (Scott object out)."""
    mu = _make_mu(_d_store(), _KERNEL.step_native(scott_to_native))
    return native_to_scott(mu(scott_to_native(e)))


# register the native table as Python's override set (the universal interface): each
# entry is the host twin of the canonical lambda definition of the same name
for _name, _fn in _NATIVE.items():
    _KERNEL.override(_name, _fn)


# ===================== reduce: mu, the ground truth =====================
"""mu, the meaning function, as a genuine least fixed point in lambda: mu = Y(tau).

Backus §13.4: mu = lfp tau. tau is ONE reduction step; the recursion of mu is supplied
by the Y combinator in lam.py, NOT by Python's call stack — there is no `def step` that
calls `step`; every recursive call (the operator, the operand, the metacomposition
unfolding) goes through the Y-provided `mu`. So "mu = lfp tau" is a fact about the code.

The only mechanism is Backus's metacomposition (paper eq. metacomp):

    (rho ⟨x1..xn⟩) : y  =  (rho x1) : ⟨⟨x1..xn⟩, y⟩

An application is the node ⟨APP, f, x⟩ (APP = a reserved sentinel atom). A value is its
own meaning. An atom fetches its DEFS cell: a registered host lambda is applied to the
reduced operand; a compiled FFP object o reduces as mu(o : x). A sequence metacomposes
on its head. Everything is lambda over lam.py's encoding down to the object values.
"""

# ---- the application node ⟨APP, f, x⟩ (APP = a reserved sentinel atom) ----
_APP_TAG = L.APPTAG                                          # a unique machinery sentinel, not an ORM value
APP = L.ATOM(_APP_TAG)
mkapp = lambda f: lambda x: L.SEQ(L.CONS(APP)(L.CONS(f)(L.CONS(x)(L.NIL))))
# is o an application node: a sequence whose head is the APP sentinel atom (match the head's
# TYPE, so a SEQ head — e.g. the metacomposition pair ⟨⟨OP,..⟩, x⟩ — is not APP)
isapp = lambda o: o(lambda v: L.FALSE)(lambda l:
          l(L.FALSE)(lambda h: lambda t:
            h(lambda v: L.TRUE if v is _APP_TAG else L.FALSE)(lambda l2: L.FALSE)(L.FALSE)))(L.FALSE)
_op  = lambda e: L.HEAD(L.TAIL(L._list(e)))                   # operator f  of ⟨APP, f, x⟩
_arg = lambda e: L.HEAD(L.TAIL(L.TAIL(L._list(e))))           # operand  x  of ⟨APP, f, x⟩


def make_mu(store_fn):
    """mu = Y(tau) over the definition store returned by store_fn() at reduction time."""
    def tau(mu):
        def step(e):
            def reduce_app():
                if not _KERNEL.consume_fuel():                  # supervision: exhaustion bottoms
                    return L.BOT
                fr = mu(_op(e))                              # reduce the operator (via mu)
                x = mu(_arg(e))                              # reduce the operand once (call-by-value):
                #   the metacomposition pass below hands x to a controlling operator as DATA, where
                #   mu would not otherwise descend — so it must already be a value, not an App node.
                def on_atom(a):
                    sd = _KERNEL.step_get(a)                    # the step's DEFS cell first: a
                    if sd is not None:                       # compiled def riding in D itself
                        return mu(mkapp(sd)(x))              # (Def. AREST / Cor. closure)
                    return (lambda res:
                        L.IF(L.FST(res))                     # then the process store
                          (lambda: (lambda cell:
                              L.IF(L.HEAD(cell))             # tag TRUE = registered host lambda
                                (lambda: mu(L.HEAD(L.TAIL(cell))(mu)(x)))       # impl(mu)(reduced operand)
                                (lambda: mu(mkapp(L.HEAD(L.TAIL(cell)))(x))))(  # compiled: mu(o : x)
                              L.SND(res)))
                          (lambda: L.BOT))(_KERNEL.FETCH(a)(store_fn()))
                on_seq = lambda l: mu(mkapp(L.HEAD(l))(       # metacomposition on the head
                    L.SEQ(L.CONS(fr)(L.CONS(x)(L.NIL)))))
                # §11.2.1/§13.3.1: every function is ⊥-preserving — a ⊥ operand short-circuits
                # before any impl or metacomposition can see ⊥ as data (ρ⊥ is the fr-match's ⊥ leg)
                return L.IF(L.ISBOT(x))(lambda: L.BOT)(lambda: fr(on_atom)(on_seq)(L.BOT))
            return L.IF(isapp(e))(lambda: reduce_app())(lambda: e)  # App reduces; a value is its meaning
        return step
    return L.Y(tau)                                          # mu = lfp tau


_MU = make_mu(_KERNEL.current)


def meaning_lambda(e):
    """mu e — reduce an FFP expression via the pure lambda kernel (mu = Y(tau)). Ground truth."""
    return _MU(e)


def apply_lambda(f, x):
    """The FFP application (f : x) via the pure lambda kernel: mu(f : x). Ground truth."""
    return _MU(mkapp(f)(x))


# The runtime evaluates via the delta fast-path (spec §4.2 / D5): observationally equal to the
# lambda kernel above, which stays the ground truth (and the equivalence oracle in test_delta).
# Stratum 1 of the polyglot debug: the evaluator choice is the ONE seam that cannot ride DEFS
# (rho is implemented by apply), so it gets the same shape one level down — named registrations,
# canonical kernel as ground truth, explicit switching. PYAREST_EVALUATOR selects at import.
import os as _os

_EVALUATORS = {"lambda": (meaning_lambda, apply_lambda),
               "delta": (delta_meaning, delta_apply)}
_ACTIVE = _os.environ.get("PYAREST_EVALUATOR", "delta")
if _ACTIVE not in _EVALUATORS:
    _ACTIVE = "delta"


def use_evaluator(name):
    global _ACTIVE
    if name not in _EVALUATORS:
        raise ValueError(f"unknown evaluator {name!r}; have {sorted(_EVALUATORS)}")
    _ACTIVE = name
    return name


def active_evaluator():
    return _ACTIVE


def meaning(e):
    return _EVALUATORS[_ACTIVE][0](e)


def apply(f, x):
    return _EVALUATORS[_ACTIVE][1](f, x)


# ===================== prims: the Backus base =====================
"""Backus's primitive functions (§11.2.3) and the controlling operators for his
combining forms (§11.2.4, §13.3.2), as lambda-terms over lam.py, registered into DEFS.

Every impl is a host lambda impl(mu)(operand): a first-order primitive ignores mu and
maps the reduced operand; a controlling operator receives ⟨⟨OP, params⟩, y⟩ (by
metacomposition) and returns the expression mu then reduces — using mu to reduce
sub-applications where the result nests them (CONS, ALPHA). Predicates return the
atoms T/F (Backus), not lambda booleans. These are the host base named by the paper;
every AREST function above is an FFP object built from them and reduced by the one mu.

Deferred (needs number-payload encoding, a separate step): arithmetic and value
comparison on numeric atoms — the reader interns every atom to a symbol index, so
+/-/</… over atom payloads are not yet meaningful. The structural base below is what
Codd theta1 and the constraint violation expressions are authored from.
"""

aT, aF = L.atom("T"), L.atom("F")
BOOL2A = lambda b: b(aT)(aF)                                  # Church bool -> the atom T / F

# operand accessors  n:o
_1 = lambda o: L.HEAD(L._list(o))
_2 = lambda o: L.HEAD(L.TAIL(L._list(o)))
_3 = lambda o: L.HEAD(L.TAIL(L.TAIL(L._list(o))))
_4 = lambda o: L.HEAD(L.TAIL(L.TAIL(L.TAIL(L._list(o)))))
_params = lambda whole: L.TAIL(L._list(whole))               # f1..fn from ⟨OP, f1..fn⟩
_seqp = lambda o: o(lambda v: L.FALSE)(lambda l: L.TRUE)(L.FALSE)

# ---- operand-shape guards: a primitive outside its stated shape is ⊥ (§11.2.3) ----
_isseq_b = lambda x: x(lambda v: L.FALSE)(lambda l: L.TRUE)(L.FALSE)          # Church: is a sequence
_pair_b  = lambda o: o(lambda v: L.FALSE)(lambda l: L.AND(L.NOT(L.LNULL(l)))(  # exactly two elements
              L.AND(L.NOT(L.LNULL(L.TAIL(l))))(L.LNULL(L.TAIL(L.TAIL(l))))))(L.FALSE)
_p_pair_xs = lambda o: L.AND(_pair_b(o))(_isseq_b(_2(o)))                        # ⟨x, ⟨…⟩⟩
_p_pair_sx = lambda o: L.AND(_pair_b(o))(_isseq_b(_1(o)))                        # ⟨⟨…⟩, x⟩
_p_pair_ss = lambda o: L.AND(_pair_b(o))(L.AND(_isseq_b(_1(o)))(_isseq_b(_2(o))))  # ⟨⟨…⟩, ⟨…⟩⟩


def _shaped(test, fn):
    return lambda mu: lambda o: L.IF(test(o))(lambda: fn(mu)(o))(lambda: L.BOT)


# ---- primitive functions (§11.2.3): impl(mu)(o) with mu ignored ----
def _p_sel(i):
    s = L.SEL(i)                                              # HEAD ∘ TAIL^(i-1), pure lambda
    return lambda mu: lambda o: s(o)

_tl    = lambda mu: lambda o: L.TL(o)
_id    = lambda mu: lambda o: o
_atom  = lambda mu: lambda o: o(lambda v: aT)(lambda l: L.IF(L.LNULL(l))(lambda: aT)(lambda: aF))(L.BOT)
_null  = lambda mu: lambda o: o(lambda v: aF)(lambda l: L.IF(L.LNULL(l))(lambda: aT)(lambda: aF))(L.BOT)
_eq    = _shaped(_pair_b, lambda mu: lambda o: BOOL2A(L.EQOBJ(_1(o))(_2(o))))
_apndl = _shaped(_p_pair_xs, lambda mu: lambda o: L.APNDL(o))
_apndr = _shaped(_p_pair_sx, lambda mu: lambda o: L.APNDR(o))
_distl = _shaped(_p_pair_xs, lambda mu: lambda o: L.DISTL(o))
_distr = _shaped(_p_pair_sx, lambda mu: lambda o: L.DISTR(o))
_len   = lambda mu: lambda o: L.LENGTH(o)
_rev   = lambda mu: lambda o: o(lambda v: L.BOT)(lambda l:
            L.SEQ(L.FOLDR(lambda h: lambda a: L.APPEND(a)(L.CONS(h)(L.NIL)))(L.NIL)(l)))(L.BOT)
_cat   = _shaped(_p_pair_ss, lambda mu: lambda o: L.SEQ(L.APPEND(L._list(_1(o)))(L._list(_2(o)))))
_not   = lambda mu: lambda o: L.IF(L.EQOBJ(o)(aT))(lambda: aF)(
            lambda: L.IF(L.EQOBJ(o)(aF))(lambda: aT)(lambda: L.BOT))              # not on T/F atoms
_isTF  = lambda v: L.OR(L.EQOBJ(v)(aT))(L.EQOBJ(v)(aF))                           # boolean domain
_and   = _shaped(_pair_b, lambda mu: lambda o: L.IF(L.AND(_isTF(_1(o)))(_isTF(_2(o))))(
            lambda: L.IF(L.AND(L.EQOBJ(_1(o))(aT))(L.EQOBJ(_2(o))(aT)))(lambda: aT)(lambda: aF))(
            lambda: L.BOT))                                                       # and:⟨p,q⟩, T/F only
_or    = _shaped(_pair_b, lambda mu: lambda o: L.IF(L.AND(_isTF(_1(o)))(_isTF(_2(o))))(
            lambda: L.IF(L.OR(L.EQOBJ(_1(o))(aT))(L.EQOBJ(_2(o))(aT)))(lambda: aT)(lambda: aF))(
            lambda: L.BOT))                                                       # or:⟨p,q⟩, T/F only
_revl  = lambda l: L.FOLDR(lambda h: lambda a: L.APPEND(a)(L.CONS(h)(L.NIL)))(L.NIL)(l)
_1r    = lambda mu: lambda o: o(lambda v: L.BOT)(lambda l: L.HEAD(_revl(l)))(L.BOT)         # last element
_tlr   = lambda mu: lambda o: o(lambda v: L.BOT)(lambda l:                                  # all but last;
            L.IF(L.LNULL(l))(lambda: L.BOT)(lambda: L.SEQ(_revl(L.TAIL(_revl(l))))))(L.BOT)  # tlr:φ = ⊥

# ---- value boundary ops (§11.2.3 arithmetic + NORMA value comparison): native ORM-typed
# values, so these are boundary operations, not lambda arithmetic. Same-type operands only. ----
_NA = object()
_pv = lambda o: o(lambda v: v)(lambda l: _NA)(_NA)          # the native value of an atom, else _NA
_numeric = lambda a, b: isinstance(a, (int, float)) and isinstance(b, (int, float)) \
    and not isinstance(a, bool) and not isinstance(b, bool)


def _p_tonum(x):
    """Arithmetic coercion, mirroring delta._p_tonum (the oracle law: both paths,
    identically): the store carries LEXICAL atoms, so a numeric-looking string
    is a number to + and kin; anything else bottoms."""
    if isinstance(x, bool) or x is _NA:
        return None
    if isinstance(x, (int, float)):
        return x
    if isinstance(x, str):
        try:
            return int(x)
        except ValueError:
            try:
                return float(x)
            except ValueError:
                return None
    return None


def _binnum(f):
    def prim(mu):
        def g(o):
            a, b = _p_tonum(_pv(_1(o))), _p_tonum(_pv(_2(o)))
            return L.atom(f(a, b)) if a is not None and b is not None else L.BOT
        return g
    return _shaped(_pair_b, prim)                                      # defined on ⟨x,y⟩ exactly
def _p_cmp(rel):
    def prim(mu):
        def g(o):
            a, b = _pv(_1(o)), _pv(_2(o))
            if a is _NA or b is _NA:
                return L.BOT
            na, nb = _p_tonum(a), _p_tonum(b)
            if na is not None and nb is not None:
                return aT if rel(na, nb) else aF                       # coerced like arithmetic (delta._p_cmp mirror)
            ok = _numeric(a, b) or type(a) is type(b)
            return (aT if rel(a, b) else aF) if ok else L.BOT
        return g
    return _shaped(_pair_b, prim)                                      # defined on ⟨x,y⟩ exactly
_add, _sub, _mul = _binnum(lambda a, b: a + b), _binnum(lambda a, b: a - b), _binnum(lambda a, b: a * b)
_ge, _gt = _p_cmp(lambda a, b: a >= b), _p_cmp(lambda a, b: a > b)
_le, _lt = _p_cmp(lambda a, b: a <= b), _p_cmp(lambda a, b: a < b)
_div   = _shaped(_pair_b, lambda mu: lambda o: (lambda a, b: L.atom(a / b)
            if (_numeric(a, b) and b != 0) else L.BOT)(_pv(_1(o)), _pv(_2(o))))  # ÷ (÷0 = ⊥)
_p_trans = lambda mu: lambda o: L.TRANS(o)
_rotl  = lambda mu: lambda o: L.ROTL(o)
_rotr  = lambda mu: lambda o: L.ROTR(o)
# apply:⟨f, x⟩ = f:x = mu(f : x) — membership is application; the one operation eq. sys performs,
# and what lets a VALUE (a transition relation, a handler) be fed into the one mu.
_apply = _shaped(_pair_b, lambda mu: lambda o: mu(mkapp(_1(o))(_2(o))))

# ---- controlling operators (§13.3.2): impl(mu)(⟨⟨OP, params⟩, y⟩) ----
# COMP  f1∘..∘fn : y = f1:(f2:(..(fn:y)))        right fold of application
_p_comp = lambda mu: lambda a: L.FOLDR(lambda f: lambda acc: mkapp(f)(acc))(_2(a))(_params(_1(a)))
# CONS  [f1..fn] : y = ⟨f1:y,..,fn:y⟩            each element reduced by mu (the result nests them);
#                                                 ⊥-collapsing: any fi:y = ⊥ makes the whole ⊥ (§11.2.1)
_p_cons = lambda mu: lambda a: L.SEQC(L.MAPL(lambda f: mu(mkapp(f)(_2(a))))(_params(_1(a))))
# CONST  x̄ : y = x   (⊥-preserving: x̄ : ⊥ = ⊥)
_p_const = lambda mu: lambda a: _2(a)(lambda v: _2(_1(a)))(lambda l: _2(_1(a)))(L.BOT)
# ALPHA  αf : ⟨y1..yn⟩ = ⟨f:y1,..,f:yn⟩          ⊥-collapsing like CONS
_p_alpha = lambda mu: lambda a: _2(a)(lambda v: L.BOT)(lambda l:
            L.SEQC(L.MAPL(lambda yi: mu(mkapp(_2(_1(a)))(yi)))(l)))(L.BOT)
# COND  (p→f;g) : y = f:y if p:y=T ; g:y if p:y=F ; else ⊥
def _p_cond(mu):
    def h(a):
        p, f, g, y = _2(_1(a)), _3(_1(a)), _4(_1(a)), _2(a)
        pv = mu(mkapp(p)(y))
        return L.IF(L.EQOBJ(pv)(aT))(lambda: mu(mkapp(f)(y)))(
               lambda: L.IF(L.EQOBJ(pv)(aF))(lambda: mu(mkapp(g)(y)))(lambda: L.BOT))
    return h
# INSERT  /f : ⟨x⟩ = x ; /f : ⟨x1..xn⟩ = f : ⟨x1, /f:⟨x2..xn⟩⟩   (recursion via mu = Y)
def _p_insert(mu):
    def h(a):
        f, whole, y = _2(_1(a)), _1(a), _2(a)
        yl = L._list(y)
        return L.IF(L.LNULL(yl))(lambda: L.BOT)(lambda:
            L.IF(L.LNULL(L.TAIL(yl)))(lambda: L.HEAD(yl))(lambda:
                # /f:⟨x1..xn⟩ = f:⟨x1, /f:⟨x2..xn⟩⟩ — reduce the tail fold FIRST (it sits in
                # data position inside the pair, where mu would not otherwise descend);
                # SEQC collapses the pair to ⊥ when the fold bottomed (§11.2.1)
                mkapp(f)(L.SEQC(L.CONS(L.HEAD(yl))(
                    L.CONS(mu(mkapp(whole)(L.SEQ(L.TAIL(yl)))))(L.NIL))))))
    return h
# WHILE  (while p f) : y = (while p f):(f:y) if p:y=T ; y if p:y=F ; else ⊥
def _p_while(mu):
    def h(a):
        p, f, whole, y = _2(_1(a)), _3(_1(a)), _1(a), _2(a)
        pv = mu(mkapp(p)(y))
        return L.IF(L.EQOBJ(pv)(aT))(lambda: mkapp(whole)(mkapp(f)(y)))(
               lambda: L.IF(L.EQOBJ(pv)(aF))(lambda: y)(lambda: L.BOT))
    return h
# BU  (bu f x) : y = f:⟨x, y⟩   — binary-to-unary (§11.2.4); x is quoted data
_p_bu = lambda mu: lambda a: mu(mkapp(_2(_1(a)))(
        L.SEQC(L.CONS(_3(_1(a)))(L.CONS(_2(a))(L.NIL)))))


def register_base():
    for i in range(1, 33):                                   # selectors 1..32 (a number is a selector)
        register(i, _p_sel(i))
    prims = {"tl": _tl, "id": _id, "atom": _atom, "null": _null, "eq": _eq,
             "apndl": _apndl, "apndr": _apndr, "distl": _distl, "distr": _distr,
             "length": _len, "reverse": _rev, "cat": _cat,
             "not": _not, "and": _and, "or": _or, "1r": _1r, "tlr": _tlr,
             "trans": _p_trans, "rotl": _rotl, "rotr": _rotr,
             "+": _add, "-": _sub, "*": _mul, "div": _div,
             "ge": _ge, "gt": _gt, "le": _le, "lt": _lt,
             "apply": _apply,
             "COMP": _p_comp, "CONS": _p_cons, "CONST": _p_const, "ALPHA": _p_alpha,
             "COND": _p_cond, "INSERT": _p_insert, "WHILE": _p_while, "BU": _p_bu}
    for name, fn in prims.items():
        register(name, fn)


register_base()

# the formal base, snapshotted: everything registered AFTER this line (bridges, cellkey,
# FFI) is beyond the paper's base language — the boundary of Cor. boundary
BASE = tuple(_KERNEL._registered)
