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
