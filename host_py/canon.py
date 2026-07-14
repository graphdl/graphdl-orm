"""The Python binding of the INTERSECTION SOURCE vocabulary (shared/*.py),
the repo's layout, and Codd's theta1 bindings (merged from paths.py and
theta.canon, 2026-07-04, the fewer-files push: one module owns how shared
sources are FOUND, LOADED, and NAMED from Python).

An intersection file is normal Python and normal Rust verbatim: statements of nested
calls over the vocabulary DEF, A, N, PHI, S2..S9, semicolon-terminated, double-quoted
strings only, no imports, no assignments, no host functions. Each platform defines
the vocabulary and consumes the same bytes; here Python's exec binds it (the lambda
used determines the implementation), the Rust host include!s the identical file into
a function whose scope defines the same names, and the C# and Java hosts wrap the same
bytes in a varargs method. Definitions land in DEFS as compiled objects and reference
each other by name through rho, so per-host OPTIMIZATIONS (delta, FAST, native) remain
DEFS registrations over the same names."""
import os

from . import defs
from .lam import atom as _atom, PHI as _PHI
from . import lam as L


# --- repo layout (rebuild, SPEC D7 + PLAN): the canon lives at the repo ROOT
# (arest.canon IS the shared codebase); hosts are host_py/, host_rs/. Hosts
# locate the canon through the repo root, never through their own package
# position. ---
_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def root():
    return _ROOT


def shared(name):
    """A canonical shared source file (the canon and readings at the root)."""
    return os.path.join(_ROOT, name)


def rust_bin(name):
    return os.path.join(_ROOT, "rust", "target", "release",
                        name + (".exe" if os.name == "nt" else ""))


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


def vocabulary_native(out):
    """The THIRD consumer of the intersection files (stratum 2 of the polyglot
    debug): the same names building the delta evaluator's carrier directly — a
    scalar IS an atom, a tuple IS a sequence — so definition resolution on the
    fast path can skip the Scott boundary entirely. Faithfulness is pinned by
    the differential against scott_to_native of the canonical load."""
    v = {"PHI": lambda: (), "A": lambda x: x, "N": lambda x: x,
         "K": lambda x: ("CONST", x),
         "DEF": lambda name, obj: out.append((name, obj))}
    for k in range(1, 10):
        def _sk(*xs, _k=k):
            if len(xs) != _k:
                raise TypeError("S%d takes %d arguments, got %d" % (_k, _k, len(xs)))
            return tuple(xs)
        v["S%d" % k] = _sk
    return v


def read(name):
    """Collect a shared intersection file's definitions without registering them."""
    p = shared(name)
    out = []
    exec(compile(open(p, encoding="utf-8").read(), p, "exec"), vocabulary(out))
    return out


def read_native(name):
    """Collect a shared file's definitions as delta-native objects (no Scott)."""
    p = shared(name)
    out = []
    exec(compile(open(p, encoding="utf-8").read(), p, "exec"), vocabulary_native(out))
    return out


def load(name="arest.canon"):
    """Consume a shared intersection file: exec under BOTH vocabularies, register
    each definition with its native twin (the fast path's store then skips the
    Scott boundary for canonical names). Returns the names, in file order."""
    pairs = read(name)
    npairs = dict(read_native(name))
    for n, obj in pairs:
        defs.define(n, obj, native_obj=npairs.get(n))
    return [n for n, _ in pairs]


def load_all():
    """Every intersection file, in dependency order (constraints, ast, and system
    reference theta; system's pipeline references ast)."""
    return load("arest.canon")


# --- Codd's adequate collection theta1 (Codd §2.2), from theta.canon: the Python
# BINDING of the canonical definitions in shared/theta.canon. The closed objects
# here ARE the canon values, loaded from the shared file; the parameterized
# constructors APPLY the canonical builders through the reducer (canon boots
# with the package, so the names resolve). Nothing is defined twice: this half
# binds, it does not author. "Navigation needs no separate query language" —
# each operator is a rho-application over the population P. ---
_C = dict(read("arest.canon"))                                  # the shared file, verbatim

member = _C["theta:member"]
dedup = _C["theta:dedup"]
flatten = _C["theta:flatten"]
setminus = _C["theta:setminus"]
Tie = _C["theta:Tie"]


def Filter(p):
    """Codd selection sigma_p: the canonical builder applied to p (shared/theta.canon)."""
    from .lam import atom as A
    from .reduce import apply as _apply
    return _apply(A("theta:Filter"), p)


def NatJoin(i):
    """Codd natural join R*S (§2.1.3), joining R.i = S.1: the canonical builder
    applied to the selector."""
    from .lam import atom as A
    from .reduce import apply as _apply
    return _apply(A("theta:NatJoin"), A(i))


def Project(cols):
    """Codd projection pi_L (§2.1.2): the canonical builder applied to the selector
    row ⟨c1..ck⟩."""
    from .lam import atom as A, to_lam
    from .reduce import apply as _apply
    return _apply(A("theta:Project"), to_lam(tuple(cols)))


def JoinOn(pairs, keep):
    """Codd's join (§2.1.3) in its general equi form: R ⋈ S on {R.ri = S.si} for the
    (ri, si) in `pairs`, emitting r ++ s[keep] (the fresh columns, in clause order).
    Empty `pairs` is the degenerate cross product; empty `keep` is the semijoin. The
    canonical COND-over-null builder applied to ⟨pairs, keep⟩ (shared/theta.canon).
        match   = eq∘[⟨ri…⟩∘1, ⟨si…⟩∘2]
        combine = cat∘[1, ⟨keep…⟩∘2]        (just 1 when keep is empty)
        R⋈S     = flatten ∘ α( α(combine) ∘ Filter(match) ∘ distl ) ∘ distr
    """
    from .lam import atom as A, to_lam
    from .reduce import apply as _apply
    return _apply(A("theta:JoinOn"),
                  to_lam((tuple(tuple(p) for p in pairs), tuple(keep))))


def Restrict(cols_L, cols_M):
    """Codd restriction R_{L|M}S (§2.1.5): the maximal R'⊆R with pi_L(R')=pi_M(S),
    over ⟨R, S⟩ — the semijoin keeping rows of R whose L-key occurs in pi_M(S). The
    canonical builder applied to ⟨L, M⟩ (shared/theta.canon).
        Restrict(L,M) = α(1) ∘ Filter(pi_L(r) ∈ pi_M(S)) ∘ distr ∘ [1, pi_M∘2]
    """
    from .lam import atom as A, to_lam
    from .reduce import apply as _apply
    return _apply(A("theta:Restrict"),
                  to_lam((tuple(cols_L), tuple(cols_M))))
