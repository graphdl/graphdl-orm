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
from . import lam as L
from .defs import register
from .reduce import mkapp

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
_pair_xs = lambda o: L.AND(_pair_b(o))(_isseq_b(_2(o)))                        # ⟨x, ⟨…⟩⟩
_pair_sx = lambda o: L.AND(_pair_b(o))(_isseq_b(_1(o)))                        # ⟨⟨…⟩, x⟩
_pair_ss = lambda o: L.AND(_pair_b(o))(L.AND(_isseq_b(_1(o)))(_isseq_b(_2(o))))  # ⟨⟨…⟩, ⟨…⟩⟩


def _shaped(test, fn):
    return lambda mu: lambda o: L.IF(test(o))(lambda: fn(mu)(o))(lambda: L.BOT)


# ---- primitive functions (§11.2.3): impl(mu)(o) with mu ignored ----
def _sel(i):
    s = L.SEL(i)                                              # HEAD ∘ TAIL^(i-1), pure lambda
    return lambda mu: lambda o: s(o)

_tl    = lambda mu: lambda o: L.TL(o)
_id    = lambda mu: lambda o: o
_atom  = lambda mu: lambda o: o(lambda v: aT)(lambda l: L.IF(L.LNULL(l))(lambda: aT)(lambda: aF))(L.BOT)
_null  = lambda mu: lambda o: o(lambda v: aF)(lambda l: L.IF(L.LNULL(l))(lambda: aT)(lambda: aF))(L.BOT)
_eq    = _shaped(_pair_b, lambda mu: lambda o: BOOL2A(L.EQOBJ(_1(o))(_2(o))))
_apndl = _shaped(_pair_xs, lambda mu: lambda o: L.APNDL(o))
_apndr = _shaped(_pair_sx, lambda mu: lambda o: L.APNDR(o))
_distl = _shaped(_pair_xs, lambda mu: lambda o: L.DISTL(o))
_distr = _shaped(_pair_sx, lambda mu: lambda o: L.DISTR(o))
_len   = lambda mu: lambda o: L.LENGTH(o)
_rev   = lambda mu: lambda o: o(lambda v: L.BOT)(lambda l:
            L.SEQ(L.FOLDR(lambda h: lambda a: L.APPEND(a)(L.CONS(h)(L.NIL)))(L.NIL)(l)))(L.BOT)
_cat   = _shaped(_pair_ss, lambda mu: lambda o: L.SEQ(L.APPEND(L._list(_1(o)))(L._list(_2(o)))))
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
def _binnum(f):
    def prim(mu):
        def g(o):
            a, b = _pv(_1(o)), _pv(_2(o))
            return L.atom(f(a, b)) if _numeric(a, b) else L.BOT        # int/float are one numeric domain
        return g
    return _shaped(_pair_b, prim)                                      # defined on ⟨x,y⟩ exactly
def _cmp(rel):
    def prim(mu):
        def g(o):
            a, b = _pv(_1(o)), _pv(_2(o))
            ok = a is not _NA and b is not _NA and (_numeric(a, b) or type(a) is type(b))
            return (aT if rel(a, b) else aF) if ok else L.BOT         # numeric ordering across int/float
        return g
    return _shaped(_pair_b, prim)                                      # defined on ⟨x,y⟩ exactly
_add, _sub, _mul = _binnum(lambda a, b: a + b), _binnum(lambda a, b: a - b), _binnum(lambda a, b: a * b)
_ge, _gt = _cmp(lambda a, b: a >= b), _cmp(lambda a, b: a > b)
_le, _lt = _cmp(lambda a, b: a <= b), _cmp(lambda a, b: a < b)
_div   = _shaped(_pair_b, lambda mu: lambda o: (lambda a, b: L.atom(a / b)
            if (_numeric(a, b) and b != 0) else L.BOT)(_pv(_1(o)), _pv(_2(o))))  # ÷ (÷0 = ⊥)
_trans = lambda mu: lambda o: L.TRANS(o)
_rotl  = lambda mu: lambda o: L.ROTL(o)
_rotr  = lambda mu: lambda o: L.ROTR(o)
# apply:⟨f, x⟩ = f:x = mu(f : x) — membership is application; the one operation eq. sys performs,
# and what lets a VALUE (a transition relation, a handler) be fed into the one mu.
_apply = _shaped(_pair_b, lambda mu: lambda o: mu(mkapp(_1(o))(_2(o))))

# ---- controlling operators (§13.3.2): impl(mu)(⟨⟨OP, params⟩, y⟩) ----
# COMP  f1∘..∘fn : y = f1:(f2:(..(fn:y)))        right fold of application
_comp = lambda mu: lambda a: L.FOLDR(lambda f: lambda acc: mkapp(f)(acc))(_2(a))(_params(_1(a)))
# CONS  [f1..fn] : y = ⟨f1:y,..,fn:y⟩            each element reduced by mu (the result nests them);
#                                                 ⊥-collapsing: any fi:y = ⊥ makes the whole ⊥ (§11.2.1)
_cons = lambda mu: lambda a: L.SEQC(L.MAPL(lambda f: mu(mkapp(f)(_2(a))))(_params(_1(a))))
# CONST  x̄ : y = x   (⊥-preserving: x̄ : ⊥ = ⊥)
_const = lambda mu: lambda a: _2(a)(lambda v: _2(_1(a)))(lambda l: _2(_1(a)))(L.BOT)
# ALPHA  αf : ⟨y1..yn⟩ = ⟨f:y1,..,f:yn⟩          ⊥-collapsing like CONS
_alpha = lambda mu: lambda a: _2(a)(lambda v: L.BOT)(lambda l:
            L.SEQC(L.MAPL(lambda yi: mu(mkapp(_2(_1(a)))(yi)))(l)))(L.BOT)
# COND  (p→f;g) : y = f:y if p:y=T ; g:y if p:y=F ; else ⊥
def _cond(mu):
    def h(a):
        p, f, g, y = _2(_1(a)), _3(_1(a)), _4(_1(a)), _2(a)
        pv = mu(mkapp(p)(y))
        return L.IF(L.EQOBJ(pv)(aT))(lambda: mu(mkapp(f)(y)))(
               lambda: L.IF(L.EQOBJ(pv)(aF))(lambda: mu(mkapp(g)(y)))(lambda: L.BOT))
    return h
# INSERT  /f : ⟨x⟩ = x ; /f : ⟨x1..xn⟩ = f : ⟨x1, /f:⟨x2..xn⟩⟩   (recursion via mu = Y)
def _insert(mu):
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
def _while(mu):
    def h(a):
        p, f, whole, y = _2(_1(a)), _3(_1(a)), _1(a), _2(a)
        pv = mu(mkapp(p)(y))
        return L.IF(L.EQOBJ(pv)(aT))(lambda: mkapp(whole)(mkapp(f)(y)))(
               lambda: L.IF(L.EQOBJ(pv)(aF))(lambda: y)(lambda: L.BOT))
    return h
# BU  (bu f x) : y = f:⟨x, y⟩   — binary-to-unary (§11.2.4); x is quoted data
_bu = lambda mu: lambda a: mu(mkapp(_2(_1(a)))(
        L.SEQC(L.CONS(_3(_1(a)))(L.CONS(_2(a))(L.NIL)))))


def register_base():
    for i in range(1, 33):                                   # selectors 1..32 (a number is a selector)
        register(i, _sel(i))
    prims = {"tl": _tl, "id": _id, "atom": _atom, "null": _null, "eq": _eq,
             "apndl": _apndl, "apndr": _apndr, "distl": _distl, "distr": _distr,
             "length": _len, "reverse": _rev, "cat": _cat,
             "not": _not, "and": _and, "or": _or, "1r": _1r, "tlr": _tlr,
             "trans": _trans, "rotl": _rotl, "rotr": _rotr,
             "+": _add, "-": _sub, "*": _mul, "div": _div,
             "ge": _ge, "gt": _gt, "le": _le, "lt": _lt,
             "apply": _apply,
             "COMP": _comp, "CONS": _cons, "CONST": _const, "ALPHA": _alpha,
             "COND": _cond, "INSERT": _insert, "WHILE": _while, "BU": _bu}
    for name, fn in prims.items():
        register(name, fn)


register_base()

# the formal base, snapshotted: everything registered AFTER this line (bridges, cellkey,
# FFI) is beyond the paper's base language — the boundary of Cor. boundary
from . import defs as _defs
BASE = tuple(_defs._registered)
