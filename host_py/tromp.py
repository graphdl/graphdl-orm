"""Tromp diagrams + the rho-fidelity checker (engine tasks 20 & 22, one deliverable).

This is the LOAD-BEARING-MATH litmus. It checks the OBJECT --- the canon DEFs and the
fact population --- against pure lambda calculus itself, not against host agreement. Two
halves, one substrate:

  * A reified pure-lambda Term algebra (De Bruijn: ``Var`` / ``Lam`` / ``App`` plus
    ``Atom`` leaves for a fact's native objects), a normal-order beta-reducer with a fuel
    bound, and a Tromp-diagram renderer (SVG + a matching ASCII renderer off ONE layout).

  * ``def_to_term``: every Backus/FFP combining form and primitive lifted to a reified
    De Bruijn term (transcribed from ``kernel.py`` + the paper), so each canon DEF lifts to
    a pure term; and ``fact_to_term``: a populated fact ``<CONS,s1..sn>`` applied to objects
    is  ``lambda f. f o1 .. on``  (paper p.2 --- "membership is application", a set IS its
    characteristic function).

The two certifications the paper's semantics demand, exactly (settled, not re-derived):

  DEFS  --- recursion is ALWAYS the Y combinator (Backus's least fixed point, sec.12.8), so a
            general DEF has NO normal form. The checker proves each DEF LIFTS to a CLOSED,
            WELL-FORMED pure term (finite syntax); it does NOT ask it to beta-normalize.
  FACTS --- a populated fact is finite and beta-reduces to  ``lambda f. f(o1..on)``. The
            checker reduces and asserts that normal form. THAT equality is rho-fidelity.

The two boundaries the paper draws are respected honestly. Backus's PRIMITIVE functions
(sec.11.2.3: the selectors, ``eq``, ``distl``, arithmetic, ...) are the delta-constant LEAVES
of the applied lambda calculus --- decidable base operations (paper sec.5.2: "arithmetic,
length, dynamic application"), part of the closed pure term. The REGISTERED host layer
(``implode``/``escape_html``/``slug``/``lex``/``strip_prefix`` and kin: arbitrary host code,
eq.(boundary), "the line at which Turing-complete computation re-enters") is NOT pure lambda;
a DEF that transitively reaches it does not lift to a closed pure term, and that is reported
plainly rather than papered over.

Self-contained: nothing here is imported by the package at boot; ``pyarest.canon`` /
``pyarest.kernel`` are imported lazily inside the functions that read the canon.
"""
from __future__ import annotations

import sys as _sys
from dataclasses import dataclass

# Lifting the whole canon builds per-DEF component terms (selectors up to ~52 deep) and the
# reducer/inspectors recurse on term depth; give the host headroom (conftest already does this
# for the test process, but the module is also usable standalone).
if _sys.getrecursionlimit() < 200_000:
    _sys.setrecursionlimit(200_000)


# ============================================================================
# 1. The reified pure-lambda Term algebra (De Bruijn indices).
# ============================================================================
# A term is one of:
#   Var(index)     a bound variable, De Bruijn index (0 = innermost binder)
#   Lam(body)      an abstraction  (lambda . body)
#   App(fn, arg)   an application  (fn arg)
#   Atom(value)    a native leaf: a fact's o_i, quoted data, or a base-primitive delta-constant.
# Frozen dataclasses give structural (alpha-)equality for free: in De Bruijn form two terms are
# alpha-equivalent iff they are structurally identical, so ``==`` IS alpha-equivalence.

class Term:
    __slots__ = ()


@dataclass(frozen=True)
class Var(Term):
    index: int


@dataclass(frozen=True)
class Lam(Term):
    body: Term


@dataclass(frozen=True)
class App(Term):
    fn: Term
    arg: Term


@dataclass(frozen=True)
class Atom(Term):
    value: object
    label: object = None            # a human string for rendering; value carries identity


# ---- well-formedness and closedness -----------------------------------------
def well_formed(t: Term) -> bool:
    """Structural well-formedness: every node is a Term of the four kinds, every Var index is a
    non-negative int. (A free index is still well-formed --- closedness is the separate test.)"""
    if isinstance(t, Var):
        return isinstance(t.index, int) and t.index >= 0
    if isinstance(t, Lam):
        return well_formed(t.body)
    if isinstance(t, App):
        return well_formed(t.fn) and well_formed(t.arg)
    if isinstance(t, Atom):
        return True
    return False


def max_free_index(t: Term, depth: int = 0) -> int:
    """The largest (free_index - depth) over the term; < 0 iff the term is CLOSED. Memoized on
    node identity so a shared sub-term (the environment inside a component letrec) is walked once."""
    memo: dict[int, int] = {}

    def go(u: Term, d: int) -> int:
        if isinstance(u, Var):
            return u.index - d                     # >=0 means it escapes d binders
        if isinstance(u, Atom):
            return -1
        key = (id(u), d)
        hit = memo.get(key)
        if hit is not None:
            return hit
        if isinstance(u, Lam):
            r = go(u.body, d + 1)
        elif isinstance(u, App):
            r = max(go(u.fn, d), go(u.arg, d))
        else:
            raise TypeError(f"not a Term: {u!r}")
        memo[key] = r
        return r

    return go(t, depth)


def is_closed(t: Term) -> bool:
    """No free De Bruijn index escapes: every Var(i) sits under at least i+1 binders."""
    return max_free_index(t, 0) < 0


def size(t: Term) -> int:
    seen: set[int] = set()

    def go(u: Term) -> int:
        if id(u) in seen:                          # count shared structure once
            return 0
        seen.add(id(u))
        if isinstance(u, (Var, Atom)):
            return 1
        if isinstance(u, Lam):
            return 1 + go(u.body)
        return 1 + go(u.fn) + go(u.arg)

    return go(t)


def to_str(t: Term) -> str:
    """A readable classic-notation printer (De Bruijn indices shown as numbers)."""
    if isinstance(t, Var):
        return str(t.index)
    if isinstance(t, Atom):
        return str(t.label if t.label is not None else t.value)
    if isinstance(t, Lam):
        return "\\." + to_str(t.body)
    f = to_str(t.fn)
    a = to_str(t.arg)
    if isinstance(t.arg, App):
        a = "(" + a + ")"
    if isinstance(t.fn, Lam):
        f = "(" + f + ")"
    return f + " " + a


# ============================================================================
# 2. Normal-order beta-reduction to normal form, fuel-bounded.
# ============================================================================
# Standard De Bruijn shift/substitution. Y-terms never reach a normal form; the fuel bound
# makes the reducer return the partial term plus a "non-normalizing" flag instead of hanging.

def shift(t: Term, d: int, cutoff: int = 0) -> Term:
    if isinstance(t, Var):
        return Var(t.index + d) if t.index >= cutoff else t
    if isinstance(t, Atom):
        return t
    if isinstance(t, Lam):
        return Lam(shift(t.body, d, cutoff + 1))
    return App(shift(t.fn, d, cutoff), shift(t.arg, d, cutoff))


def subst(t: Term, j: int, s: Term) -> Term:
    """t[j := s]  (De Bruijn), used by beta with j growing under binders."""
    if isinstance(t, Var):
        return s if t.index == j else t
    if isinstance(t, Atom):
        return t
    if isinstance(t, Lam):
        return Lam(subst(t.body, j + 1, shift(s, 1, 0)))
    return App(subst(t.fn, j, s), subst(t.arg, j, s))


def beta(lam: Lam, arg: Term) -> Term:
    """(\\. body) arg  -->  body[0 := arg], with the usual shifts."""
    return shift(subst(lam.body, 0, shift(arg, 1, 0)), -1, 0)


def _step(t: Term):
    """One leftmost-outermost (normal-order) beta step. Returns (t', reduced?)."""
    if isinstance(t, App):
        if isinstance(t.fn, Lam):
            return beta(t.fn, t.arg), True                  # the leftmost-outermost redex
        f2, red = _step(t.fn)
        if red:
            return App(f2, t.arg), True
        a2, red = _step(t.arg)
        if red:
            return App(t.fn, a2), True
        return t, False
    if isinstance(t, Lam):
        b2, red = _step(t.body)                             # reduce under the binder -> full NF
        return (Lam(b2), red) if red else (t, False)
    return t, False


DEFAULT_FUEL = 100_000


def normalize(t: Term, fuel: int = DEFAULT_FUEL):
    """Reduce to beta-normal form under a fuel bound.

    Returns (term, normal) where ``normal`` is True iff a normal form was reached within fuel.
    On exhaustion the partially-reduced term is returned with normal=False --- a Y-term (every
    general DEF) lands here, and the reducer never crashes or hangs."""
    steps = 0
    while steps < fuel:
        t, red = _step(t)
        if not red:
            return t, True
        steps += 1
    return t, False


# ============================================================================
# 3. A HOAS builder: write kernel-shaped ``lambda a: lambda b: ...`` and get De Bruijn out.
# ============================================================================
# ``_B`` wraps a function  depth -> Term ; ``__call__`` is application, so a transcription reads
# almost verbatim against kernel.py. ``Lm(lambda x: body)`` binds x and yields its Var at the
# right index automatically. Finalise with ``_run``.

class _B:
    __slots__ = ("build",)

    def __init__(self, build):
        self.build = build

    def __call__(self, other):
        o = _asB(other)
        return _B(lambda d: App(self.build(d), o.build(d)))


def _asB(x) -> _B:
    if isinstance(x, _B):
        return x
    if isinstance(x, Term):
        return _B(lambda d: x)
    raise TypeError(f"cannot lift {x!r} into a term builder")


def Lm(f) -> _B:
    """Bind one variable: f receives the new variable (as a builder) and returns the body."""
    def build(d):
        d2 = d + 1
        xvar = _B(lambda dd, _lvl=d2: Var(dd - _lvl))
        return Lam(_asB(f(xvar)).build(d2))
    return _B(build)


def At(value, label=None) -> _B:
    return _B(lambda d: Atom(value, label))


def _run(b: _B) -> Term:
    return _asB(b).build(0)


# ============================================================================
# 4. The pure-lambda substrate, transcribed from kernel.py (lines 29-140).
# ============================================================================
# Only the machinery the combining forms and selectors are built from is transcribed here as
# genuine terms; Backus's PRIMITIVE functions themselves are delta-constant leaves (Section 6).

I   = Lm(lambda x: x)                                              # identity (kernel L.I)
K   = Lm(lambda x: Lm(lambda y: x))                               # K / TRUE (a boolean IS its selector)
TRUE  = Lm(lambda a: Lm(lambda b: a))                            # kernel line 34
FALSE = Lm(lambda a: Lm(lambda b: b))                            # kernel line 35
# Y = lfp (kernel line 31):  \f. (\x. f (\v. x x v)) (\x. f (\v. x x v))
Y = Lm(lambda f: (Lm(lambda x: f(Lm(lambda v: x(x)(v)))))
                 (Lm(lambda x: f(Lm(lambda v: x(x)(v))))))

# the Scott object union  ATOM v | SEQ l | BOT  and Scott lists (kernel lines 53-63)
BOT   = Lm(lambda a: Lm(lambda s: Lm(lambda b: b)))              # match onBot
SEQ   = Lm(lambda l: Lm(lambda a: Lm(lambda s: Lm(lambda b: s(l)))))
NIL   = Lm(lambda n: Lm(lambda c: n))
CONSL = Lm(lambda h: Lm(lambda t: Lm(lambda n: Lm(lambda c: c(h)(t)))))
HEAD  = Lm(lambda l: l(BOT)(Lm(lambda h: Lm(lambda t: h))))
TAIL  = Lm(lambda l: l(NIL)(Lm(lambda h: Lm(lambda t: t))))
# apply-to-all's engine (kernel line 67): MAPL f <x1..xn> = <f x1 .. f xn>
MAPL  = Y(Lm(lambda rec: Lm(lambda f: Lm(lambda l:
          l(NIL)(Lm(lambda h: Lm(lambda t: CONSL(f(h))(rec(f)(t)))))))))


# ---- the Church tuple  <o1..on> = \f. f o1 .. on  (paper p.2) ----------------
def _tuple(elems):
    """The n-ary Church tuple as a builder:  \\f. f e1 e2 .. en  (n=0 gives \\f. f)."""
    def body(f):
        acc = f
        for e in elems:
            acc = acc(e)
        return acc
    return Lm(body)


PHI = _tuple([])                                                  # the empty sequence  <> = \f. f


# ============================================================================
# 5. The Backus combining forms (sec.11.2.4), lifted to reified De Bruijn terms.
# ============================================================================
# Denotational transcription (equivalent to kernel.py's controlling operators _p_comp/_p_cons/
# ...): each form is a genuine closed lambda term over the substrate above. INSERT / ALPHA /
# WHILE use Y --- finite closed terms that simply never normalize, exactly as the doctrine says.

def form_comp(fns):
    """COMP  f1.f2...fn : y = f1:(f2:(..(fn:y)))   right-fold of application (kernel _p_comp)."""
    if not fns:
        return I

    def body(y):
        acc = y
        for f in reversed(fns):
            acc = f(acc)
        return acc
    return Lm(body)


def form_cons(fns):
    """CONS  [f1..fn] : y = <f1:y,..,fn:y>   the Church tuple of the applications (kernel _p_cons)."""
    return Lm(lambda y: _tuple([f(y) for f in fns]))


def form_const(dataterm):
    """CONST  x_bar : y = x   (K x; the payload is quoted DATA, already a closed term)."""
    return Lm(lambda _y: _asB(dataterm))


def form_cond(p, f, g):
    """COND  (p->f;g) : y   --- a predicate IS its selector, so (p y) chooses (f y) / (g y)."""
    return Lm(lambda y: p(y)(f(y))(g(y)))


def form_alpha(f):
    """ALPHA  af : <y1..yn> = <f:y1,..,f:yn>   (MAPL over the operand; Y-recursive)."""
    return Lm(lambda xs: MAPL(f)(xs))


def form_insert(f):
    """INSERT  /f : <x> = x ; /f : <x1..xn> = f:<x1,/f:<x2..xn>>   (right reduce; recursion via Y).

    Transcribed from kernel _p_insert: over a Scott list, if the tail is NIL the single element
    is the result, else  f<head, rec tail>.  A finite closed Y-term that does not normalize."""
    reducer = Y(Lm(lambda rec: Lm(lambda l:
        l(BOT)(Lm(lambda h: Lm(lambda t:
            t(h)(Lm(lambda _h2: Lm(lambda _t2: f(_tuple([h, rec(t)]))))))))) ))
    return Lm(lambda xs: reducer(xs))


def form_while(p, f):
    """WHILE  (while p f) : y = (while p f):(f:y) if p:y ; y otherwise   (Y-recursive loop)."""
    loop = Y(Lm(lambda rec: Lm(lambda x: p(x)(rec(f(x)))(x))))
    return Lm(lambda y: loop(y))


def form_bu(f, c):
    """BU  (bu f x) : y = f:<x, y>   binary-to-unary; x is quoted DATA (kernel _p_bu)."""
    return Lm(lambda y: f(_tuple([_asB(c), y])))


def selector(n: int):
    """selector n : <o1..om> = o_n  =  HEAD . TAIL^(n-1)  over the Scott list inside a SEQ
    (kernel SEL, lines 95-101). A genuine pure term (not a delta-constant)."""
    def onseq(l):
        acc = l
        for _ in range(n - 1):
            acc = TAIL(acc)
        return HEAD(acc)
    return Lm(lambda o: o(Lm(lambda _v: BOT))(Lm(onseq))(BOT))


# The combining forms, by canon head name.
_FORMS = {"COMP", "CONS", "COND", "CONST", "ALPHA", "INSERT", "WHILE", "BU"}


# ============================================================================
# 6. Backus's PRIMITIVE functions (sec.11.2.3): the delta-constant leaves.
# ============================================================================
# These are primitives of the base, not programs built from it --- the leaves of the applied
# lambda calculus (kernel.py implements them as native ops over the value boundary / Scott
# objects). Representing each as an atomic constant is faithful to Backus and keeps the lift a
# genuine closed term. The set is exactly kernel's registered base (BASE) minus the forms and
# the selectors (selectors are transcribed above); it is discovered from the kernel at load,
# so it can never silently drift from the host.

def _prim_leaf(name):
    return Atom(("prim", name), label=name)


# ============================================================================
# 7. fact_to_term --- a populated fact as the pure term  \f. f o1 .. on  (paper p.2).
# ============================================================================

def fact_to_term(objects) -> Term:
    """A populated fact  <CONS,s1..sn>  applied to concrete objects o1..on is the Church tuple

        lambda f. f o1 .. on

    i.e. Lam(App(...App(Var0, Atom(o1))..., Atom(on))). Applying it to a selector projects a
    component --- membership IS application. Each o_i becomes an Atom leaf (its native value)."""
    atoms = [obj if isinstance(obj, Term) else Atom(obj, label=str(obj)) for obj in objects]
    body: Term = Var(0)                                           # f, under the outer binder
    for a in atoms:
        body = App(body, shift(a, 1, 0))                         # objects are closed, shift is a no-op
    return Lam(body)


# a convenience: the canonical projections used to demonstrate membership=application
def church_selector(i: int, n: int) -> Term:
    """The pure projection  \\x1..xn. x_i  (1-based). A fact of arity n applied to it yields o_i."""
    return _run(_church_selector_b(i, n))


def _church_selector_b(i: int, n: int) -> _B:
    def nest(k):
        if k > n:
            return _var_at(n - i)                                # x_i counting from the innermost
        return Lm(lambda _x, _k=k: nest(k + 1))
    return nest(1)


def _var_at(idx):
    return _B(lambda d: Var(idx))


# ============================================================================
# 8. Reading the canon and lifting each DEF to a reified pure term.
# ============================================================================

def _load_canon():
    """(pairs, canon_names, base_prims) --- the DEFs as delta-native trees, the DEF-name set, and
    the base-primitive name set (kernel BASE minus forms and integer selectors)."""
    from . import canon as _canon
    from . import kernel as _kernel
    pairs = _canon.read_native("arest.canon")                    # [(name, native tree)], file order
    canon_names = {n for n, _ in pairs}
    base_prims = {b for b in _kernel.BASE
                  if not isinstance(b, int) and b not in _FORMS}
    return pairs, canon_names, base_prims


class _Ref:
    """A temporary marker for a canon-DEF cross-reference (resolved into the component letrec)."""
    __slots__ = ("name",)

    def __init__(self, name):
        self.name = name


class _Host:
    """A temporary marker for a reference the pure base does NOT provide: a registered host
    definition or an otherwise-unknown atom. It becomes a FREE variable, so the lifted term is
    honestly not closed --- the genuine finding about a DEF that reaches the boundary."""
    __slots__ = ("name",)

    def __init__(self, name):
        self.name = name


def _lift_data(o) -> Term:
    """Quoted DATA (a CONST/K payload, a BU-bound value, any element under them). An atom is an
    Atom leaf; a sequence is the Church tuple of its lifted elements --- the same shape as a fact."""
    if isinstance(o, tuple):
        return _run(_tuple([_asB(_lift_data(e)) for e in o]))
    return Atom(o, label=str(o))


def _lift_fn(o, canon_names, base_prims):
    """Lift o in FUNCTION position to a builder (``_B``). Cross-references and boundary atoms are
    left as _Ref / _Host markers (wrapped as Atom sentinels) and resolved by ``_resolve``."""
    if isinstance(o, tuple):
        if not o:
            return _asB(BOT)                                     # empty operator is bottom
        head = o[0]
        args = o[1:]
        if head == "CONST":
            return form_const(_lift_data(args[0]) if args else PHI)
        if head == "COMP":
            return form_comp([_lift_fn(a, canon_names, base_prims) for a in args])
        if head == "CONS":
            return form_cons([_lift_fn(a, canon_names, base_prims) for a in args])
        if head == "COND":
            fs = [_lift_fn(a, canon_names, base_prims) for a in args]
            return form_cond(fs[0], fs[1], fs[2])
        if head == "ALPHA":
            return form_alpha(_lift_fn(args[0], canon_names, base_prims))
        if head == "INSERT":
            return form_insert(_lift_fn(args[0], canon_names, base_prims))
        if head == "WHILE":
            fs = [_lift_fn(a, canon_names, base_prims) for a in args]
            return form_while(fs[0], fs[1])
        if head == "BU":
            return form_bu(_lift_fn(args[0], canon_names, base_prims), _lift_data(args[1]))
        # Any other sequence head would be higher-order metacomposition; the canon never does this
        # in function position (verified: every function-position head is one of the eight forms).
        raise ValueError(f"function-position sequence not headed by a combining form: {head!r}")
    # an atom in function position
    if isinstance(o, int):
        return _asB(selector(o))                                 # a number IS a selector
    if o in canon_names:
        return _B(lambda d: Atom(("__ref__", o)))               # marker; resolved into the letrec
    if o in base_prims:
        return _B(lambda d: _prim_leaf(o))                      # a Backus base primitive (leaf)
    return _B(lambda d: Atom(("__host__", o)))                  # registered / unknown -> free var


# ---- the reference graph, computed once ----
def _fn_atoms(o, acc):
    """Every function-position leaf atom of a native tree (skips CONST payloads / BU data arg)."""
    if isinstance(o, tuple):
        if not o:
            return
        head = o[0]
        if head == "CONST":
            return
        if head == "BU":
            if len(o) > 1:
                _fn_atoms(o[1], acc)
            return
        for e in o[1:]:
            _fn_atoms(e, acc)
    else:
        acc.add(o)


_HUGE_FREE = 10_000_000                                          # an index guaranteed to stay free


def _resolve(term_with_markers: Term, order, selectors_by_pos):
    """Turn the __ref__ / __host__ Atom sentinels in a single lifted body into De Bruijn vars,
    tracking binder depth. ``order`` maps a DEF name to its 1-based slot in the component tuple;
    ``self`` is the variable bound by the letrec's outer lambda. A __host__ marker becomes a free
    variable (index _HUGE_FREE + depth) so the resulting term is provably not closed."""
    n = len(order)

    def go(u: Term, d: int) -> Term:
        if isinstance(u, Atom):
            if isinstance(u.value, tuple) and len(u.value) == 2:
                tag, nm = u.value
                if tag == "__ref__":
                    # A sibling DEF is  (self sel_j): the fixpoint self reduces to the c-tuple
                    # \f. f B_1..B_c, and applying it to sel_j = \x1..xc.xj yields B_j. ``self`` is
                    # Var(1) at a body's top (index 0 = f, index 1 = self) and rises by one per
                    # local binder, so with d starting at 1 the self index is exactly d.
                    self_idx = d
                    proj = selectors_by_pos[order[nm]]          # \x1..xc. xj  (closed -> no shift)
                    return App(Var(self_idx), proj)
                if tag == "__host__":
                    return Var(_HUGE_FREE + d)                  # free by construction
            return u
        if isinstance(u, Var):
            return u
        if isinstance(u, Lam):
            return Lam(go(u.body, d + 1))
        return App(go(u.fn, d), go(u.arg, d))

    return go(term_with_markers, 1)                            # body top sits under lambda-self, lambda-f


def _component_selectors(n):
    """The n projections over a Church n-tuple:  proj_j = \\x1..xn. x_j  (1-based)."""
    return {j: church_selector(j, n) for j in range(1, n + 1)}


class CanonLift:
    """Lifts the whole canon once and answers per-DEF questions. Build with ``CanonLift.load()``."""

    def __init__(self, pairs, canon_names, base_prims):
        self.pairs = pairs
        self.order = [n for n, _ in pairs]
        self.body = dict(pairs)
        self.canon_names = canon_names
        self.base_prims = base_prims
        # reference graph (function position only)
        self.refs = {}
        self.hostrefs = {}
        for name, tree in pairs:
            atoms = set()
            _fn_atoms(tree, atoms)
            self.refs[name] = {a for a in atoms if a in canon_names}
            self.hostrefs[name] = {a for a in atoms
                                   if not isinstance(a, int)
                                   and a not in canon_names and a not in base_prims}
        self._reach_cache = {}

    @classmethod
    def load(cls):
        return cls(*_load_canon())

    # ---- reachability over the reference graph ----
    def reachable(self, name):
        hit = self._reach_cache.get(name)
        if hit is not None:
            return hit
        seen = set()
        stack = [name]
        while stack:
            x = stack.pop()
            if x in seen:
                continue
            seen.add(x)
            stack.extend(self.refs.get(x, ()))
        self._reach_cache[name] = seen
        return seen

    def boundary_atoms(self, name):
        """The registered/unknown atoms this DEF transitively reaches (empty iff pure-closable)."""
        out = set()
        for m in self.reachable(name):
            out |= self.hostrefs.get(m, set())
        return out

    def is_pure(self, name):
        return not self.boundary_atoms(name)

    def is_recursive(self, name):
        return any(name in self.reachable(m) for m in self.refs.get(name, ()))

    # ---- the lift itself ----
    def def_to_term(self, name) -> Term:
        """Lift canon DEF ``name`` to a reified pure term.

        * A leaf DEF (no cross-references, no boundary atom) lifts to a small self-contained
          closed term --- its body directly.
        * A DEF with cross-references lifts to a projection out of a Y-letrec over its reachable
          component (recursion --- self or mutual --- tied by the ONE Y, per the doctrine).
        * A DEF that transitively reaches the registered boundary lifts to a term containing a
          free variable: honestly NOT closed. The offending atoms are ``boundary_atoms(name)``.
        """
        comp = self.reachable(name)
        boundary = self.boundary_atoms(name)
        refs = self.refs.get(name, set())
        # fast path: a self-contained leaf with a fully pure body
        if not refs and not boundary and not self.is_recursive(name):
            body_marked = _run(_lift_fn(self.body[name], self.canon_names, self.base_prims))
            return _strip_markers_pure(body_marked)
        return self._letrec(name, comp)

    def _letrec(self, name, comp) -> Term:
        members = [m for m in self.order if m in comp]            # file order, deterministic
        pos = {m: i + 1 for i, m in enumerate(members)}
        c = len(members)
        sels = _component_selectors(c)
        # lift each member body, resolve its markers against this component
        resolved = []
        for m in members:
            marked = _run(_lift_fn(self.body[m], self.canon_names, self.base_prims))
            resolved.append(_resolve(marked, pos, sels))
        # tuple body:  lambda f. f B_1 .. B_c   (f = Var0 at the head)
        tup: Term = Var(0)
        for b in resolved:
            tup = App(tup, b)
        tup_lam = Lam(tup)                                        # lambda f. f B_1 .. B_c
        self_lam = Lam(tup_lam)                                   # lambda self. lambda f. ...
        env = App(_run(Y), self_lam)                             # Y (lambda self. ...) -> the c-tuple
        return App(env, sels[pos[name]])                         # (tuple sel_j) = this DEF's slot


def _strip_markers_pure(t: Term) -> Term:
    """For a leaf DEF there are no __ref__/__host__ markers (checked by the fast-path guard); this
    is the identity, present only to make that invariant explicit."""
    return t


# ============================================================================
# 9. THE CHECKER.
# ============================================================================

class CheckReport:
    def __init__(self):
        self.total = 0
        self.pure_closed = []          # names that lifted to closed well-formed pure terms
        self.boundary = []             # (name, sorted offending atoms) -- reach the registered boundary
        self.malformed = []            # (name, reason) -- unexpected: not well-formed, or pure-but-open
        self.recursive = []            # names whose lift needed Y for name-recursion
        self.fact_checks = []          # (objects, ok) for the sample facts

    def summary(self):
        return (f"{len(self.pure_closed)}/{self.total} canon DEFs lift to CLOSED, WELL-FORMED "
                f"pure lambda terms; {len(self.boundary)} reach the registered host boundary; "
                f"{len(self.malformed)} unexpected.")


def check_canon(lift: CanonLift | None = None, verbose: bool = False) -> CheckReport:
    """Iterate EVERY canon DEF, lift it, and certify CLOSED + WELL-FORMED. A DEF that reaches the
    registered host boundary is recorded (its lift is legitimately open); anything else that fails
    to be closed & well-formed is an unexpected malformation and recorded separately."""
    if lift is None:
        lift = CanonLift.load()
    rep = CheckReport()
    rep.total = len(lift.order)
    for name in lift.order:
        term = lift.def_to_term(name)
        wf = well_formed(term)
        closed = is_closed(term)
        pure = lift.is_pure(name)
        if lift.is_recursive(name):
            rep.recursive.append(name)
        if pure:
            if wf and closed:
                rep.pure_closed.append(name)
            else:
                rep.malformed.append((name, f"pure but well_formed={wf} closed={closed}"))
        else:
            if wf and not closed:
                rep.boundary.append((name, sorted(map(str, lift.boundary_atoms(name)))))
            else:
                rep.malformed.append((name, f"boundary DEF but well_formed={wf} closed={closed}"))
        if verbose:
            print(f"  {name:32s} wf={wf} closed={closed} pure={pure} size={size(term)}")
    return rep


# sample populated facts for the reduction (rho-fidelity) certification
SAMPLE_FACTS = [
    ("Person", "has", "Name"),           # a ternary elementary fact
    ("Alice", "Bob"),                    # a binary fact
    ("Widget",),                         # a unary fact
]


def check_facts(rep: CheckReport | None = None) -> CheckReport:
    """For each sample fact, build  \\f. f o1..on , reduce it, and assert the normal form equals
    the canonical  \\f. f o1..on . Also demonstrate membership=application by projecting o_1."""
    if rep is None:
        rep = CheckReport()
    for objs in SAMPLE_FACTS:
        term = fact_to_term(objs)
        nf, normal = normalize(term)
        canonical = fact_to_term(objs)                            # the canonical target
        ok = normal and nf == canonical and is_closed(term) and well_formed(term)
        # membership is application: fact applied to the i-th projection yields o_i
        proj_ok = True
        n = len(objs)
        for i, obj in enumerate(objs, start=1):
            got, gnormal = normalize(App(term, church_selector(i, n)))
            proj_ok = proj_ok and gnormal and got == Atom(obj, label=str(obj))
        rep.fact_checks.append((objs, ok and proj_ok))
    return rep


# ============================================================================
# 10. The Tromp diagram: ONE layout model, rendered to both SVG and ASCII.
# ============================================================================
# Tromp's lambda diagrams:
#   * an abstraction (lambda) is a horizontal BAR spanning its body;
#   * a variable is a VERTICAL line rising to the bar of its binder;
#   * an application JOINS the two sub-diagrams with a horizontal link at the bottom, the result
#     continuing down from the left (function) sub-diagram.
# ``layout`` produces integer-grid segments once; the SVG and ASCII renderers consume the same
# lists, so the ASCII is a faithful low-res view of the SVG (eyeball-verifiable in-terminal).

class _Layout:
    def __init__(self):
        self.hbars = []        # (y, x1, x2)  horizontal: lambda bars AND application links
        self.vlines = []       # (x, y1, y2)  vertical:   variables and connectors
        self.labels = []       # (x, y, text) atom leaves
        self.width = 0
        self.height = 0


def layout(term: Term, gap: int = 1) -> _Layout:
    """Absolute-coordinate layout (x right, y down). Lambda bars are placed as we descend;
    variable/output lines are extended downward by the enclosing application."""
    lay = _Layout()
    # a node returns (width, out_x, bottom_y): out_x is the column of its output line at bottom_y.

    def go(t: Term, x0: int, top: int, binder_ys):
        if isinstance(t, Lam):
            bar_y = top
            w, out_x, bot = go(t.body, x0, top + 2, [bar_y] + list(binder_ys))
            lay.hbars.append((bar_y, x0, x0 + max(w, 1) - 1))
            return w, out_x, bot
        if isinstance(t, App):
            wf, ox_f, bf = go(t.fn, x0, top, binder_ys)
            ax0 = x0 + wf + gap
            wa, ox_a, ba = go(t.arg, ax0, top, binder_ys)
            base = max(bf, ba) + 1
            lay.vlines.append((ox_f, bf, base))                  # extend function output down
            lay.vlines.append((ox_a, ba, base))                  # extend argument output down
            lay.hbars.append((base, ox_f, ox_a))                 # the application link
            return wf + gap + wa, ox_f, base
        if isinstance(t, Var):
            if t.index < len(binder_ys):
                by = binder_ys[t.index]
            else:
                by = top - 1                                     # a free var: a short stub above
            lay.vlines.append((x0, by, top))
            return 1, x0, top
        # Atom leaf
        text = str(t.label if t.label is not None else t.value)
        lay.vlines.append((x0, top, top))
        lay.labels.append((x0, top, text))
        return max(1, len(text)), x0, top

    w, _ox, bot = go(term, 0, 0, [])
    lay.width = w
    lay.height = bot + 1
    return lay


def render_ascii(term: Term) -> str:
    """A low-resolution terminal view off the SAME layout as the SVG. Bars are '_' , verticals
    '|', crossings '+', atom labels printed beneath their leaf."""
    lay = layout(term)
    scale_x = 3
    W = (lay.width + 2) * scale_x
    H = lay.height + 3                                            # +1 row under the deepest for labels
    grid = [[" "] * (W + 4) for _ in range(H + 2)]

    def put(x, y, ch):
        if 0 <= y < len(grid) and 0 <= x < len(grid[0]):
            cur = grid[y][x]
            if cur == "|" and ch == "-":
                grid[y][x] = "+"
            elif cur == "-" and ch == "|":
                grid[y][x] = "+"
            else:
                grid[y][x] = ch

    for (y, x1, x2) in lay.hbars:
        yy = y
        for xx in range(x1 * scale_x, x2 * scale_x + 1):
            put(xx, yy, "-")
    for (x, y1, y2) in lay.vlines:
        xx = x * scale_x
        for yy in range(min(y1, y2), max(y1, y2) + 1):
            put(xx, yy, "|")
    for (x, y, text) in lay.labels:
        for k, ch in enumerate(text):
            put(x * scale_x + k, y + 1, ch)
    lines = ["".join(row).rstrip() for row in grid]
    while lines and lines[-1] == "":
        lines.pop()
    return "\n".join(lines)


def render_svg(term: Term, cell: int = 18, pad: int = 12, stroke: int = 3) -> str:
    """Emit a self-contained SVG Tromp diagram off the same layout (abstraction = horizontal bar,
    variable = vertical line to its binder's bar, application = the join beneath)."""
    lay = layout(term)
    W = lay.width * cell + 2 * pad
    H = lay.height * cell + 2 * pad + cell

    def px(x):
        return pad + x * cell

    def py(y):
        return pad + y * cell

    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" '
             f'viewBox="0 0 {W} {H}" font-family="monospace" font-size="{cell-4}">',
             f'<rect width="{W}" height="{H}" fill="white"/>']
    for (y, x1, x2) in lay.hbars:
        parts.append(f'<line x1="{px(x1)}" y1="{py(y)}" x2="{px(x2)}" y2="{py(y)}" '
                     f'stroke="black" stroke-width="{stroke}" stroke-linecap="round"/>')
    for (x, y1, y2) in lay.vlines:
        parts.append(f'<line x1="{px(x)}" y1="{py(y1)}" x2="{px(x)}" y2="{py(y2)}" '
                     f'stroke="black" stroke-width="{stroke}" stroke-linecap="round"/>')
    for (x, y, text) in lay.labels:
        parts.append(f'<text x="{px(x)}" y="{py(y)+cell}" fill="#1560c4">'
                     f'{_svg_escape(text)}</text>')
    parts.append("</svg>")
    return "\n".join(parts)


def _svg_escape(s):
    return (str(s).replace("&", "&amp;").replace("<", "&lt;")
            .replace(">", "&gt;").replace('"', "&quot;"))


def write_fact_svg(objects, path) -> str:
    """Render a populated fact's Tromp diagram to ``path``; return the SVG string."""
    svg = render_svg(fact_to_term(objects))
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(svg)
    return svg


# ============================================================================
# 11. A CLI for eyeballing / running the checker by hand.
# ============================================================================
def _main(argv):
    lift = CanonLift.load()
    rep = check_canon(lift)
    check_facts(rep)
    print(rep.summary())
    if rep.boundary:
        print("\nDEFs that reach the registered host boundary (NOT closed pure terms):")
        for name, atoms in rep.boundary:
            print(f"  {name:34s} -> {', '.join(atoms)}")
    if rep.malformed:
        print("\nUNEXPECTED malformations:")
        for name, why in rep.malformed:
            print(f"  {name:34s} {why}")
    print("\nrecursion (needed Y):", ", ".join(rep.recursive) or "(none)")
    print("\nfact rho-fidelity:")
    for objs, ok in rep.fact_checks:
        print(f"  {'OK ' if ok else 'BAD'} <{','.join(objs)}>  ->  lambda f. f {' '.join(objs)}")
    print("\nsample ASCII Tromp diagram --- fact <Alice,Bob>:")
    print(render_ascii(fact_to_term(("Alice", "Bob"))))
    print("\nsample ASCII Tromp diagram --- S = \\.\\.\\. (2 0)(1 0):")
    S = _run(Lm(lambda f: Lm(lambda g: Lm(lambda x: f(x)(g(x))))))
    print(render_ascii(S))


if __name__ == "__main__":
    _main(_sys.argv[1:])
