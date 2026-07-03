"""The Python binding of the INTERSECTION SOURCE vocabulary (shared/*.py).

An intersection file is normal Python and normal Rust verbatim: statements of nested
calls over the vocabulary DEF, A, N, PHI, S2..S9, semicolon-terminated, double-quoted
strings only, no imports, no assignments, no host functions. Each platform defines
the vocabulary and consumes the same bytes; here Python's exec binds it (the lambda
used determines the implementation), the Rust host include!s the identical file into
a function whose scope defines the same names, and a .NET or Java host wraps the same
bytes in a method. Definitions land in DEFS as compiled objects and reference each
other by name through rho, so per-host OPTIMIZATIONS (delta, FAST, native) remain
DEFS registrations over the same names."""
from . import defs, paths
from .lam import atom as _atom, PHI as _PHI
from . import lam as L


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def vocabulary(out):
    v = {"PHI": lambda: _PHI, "A": _atom, "N": _atom,
         "K": lambda x: _S(_atom("CONST"), x),
         "DEF": lambda name, obj: out.append((name, obj))}
    for k in range(1, 10):
        def _sk(*xs, _k=k):
            if len(xs) != _k:
                raise TypeError("S%d takes %d arguments, got %d" % (_k, _k, len(xs)))
            return _S(*xs)
        v["S%d" % k] = _sk
    return v


def read(name):
    """Collect a shared intersection file's definitions without registering them."""
    p = paths.shared(name)
    out = []
    exec(compile(open(p, encoding="utf-8").read(), p, "exec"), vocabulary(out))
    return out


def load(name="theta.py"):
    """Consume a shared intersection file: exec under the vocabulary, register every
    definition into DEFS (compiled). Returns the names, in file order."""
    pairs = read(name)
    for n, obj in pairs:
        defs.define(n, obj)
    return [n for n, _ in pairs]


def load_all():
    """Every intersection file, in dependency order (constraints and ast reference
    theta)."""
    return load("theta.py") + load("constraints.py") + load("ast.py")
