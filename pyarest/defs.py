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


def boundary():
    """Cor. boundary: the registered definitions — the informal surface of the system."""
    return list(_registered)
