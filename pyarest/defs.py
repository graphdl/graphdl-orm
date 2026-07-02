"""DEFS — the definition store (Backus §13.3.5), a lambda Scott list of cells.

A cell is ⟨key, tag, impl⟩ (a Scott list): tag TRUE = a *registered* host lambda of
the form impl(mu)(operand) — the enumerable boundary (Cor. boundary); tag FALSE = a
*compiled* FFP object o, whose meaning is mu(o : x). fetch ↑ is a lambda over the list.
register/define build the list host-side (the store is mutable state, the paper's D);
everything the reducer touches — FETCH, the cells, the keys — is lambda.
"""
from . import lam as L

_store = L.NIL            # the current Scott list of cells (newest first)
_registered = []          # names of registered (boundary) defs — host-side mirror for boundary()
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


def define(name, obj):
    """Compile: bind `name` to an FFP object o; its meaning is mu(o : x)  (Def. reg)."""
    global _store, version
    _store = L.CONS(_cell(name, L.FALSE, obj))(_store)
    compiled[name] = obj
    latest[name] = ("compiled", obj)
    version += 1


def current():
    return _store


def reset():
    global _store, _registered, compiled, latest, version
    _store, _registered, compiled, latest, version = L.NIL, [], {}, {}, version + 1


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
    """Bind D for one AST step: `with defs.step(D): …` — nestable, restored on exit."""
    def __init__(self, D):
        self._frame = (_cells_of(D), {"native": None}, D)

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
    from . import theta as T
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
