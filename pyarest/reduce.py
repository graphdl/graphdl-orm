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
from . import lam as L
from . import defs

# ---- the application node ⟨APP, f, x⟩ (APP = a reserved sentinel atom) ----
_APP_TAG = ("#APP#",)                                        # a unique machinery sentinel, not an ORM value
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
                fr = mu(_op(e))                              # reduce the operator (via mu)
                x = mu(_arg(e))                              # reduce the operand once (call-by-value):
                #   the metacomposition pass below hands x to a controlling operator as DATA, where
                #   mu would not otherwise descend — so it must already be a value, not an App node.
                on_atom = lambda a: (lambda res:
                    L.IF(L.FST(res))                         # DEFS has a cell for atom a?
                      (lambda: (lambda cell:
                          L.IF(L.HEAD(cell))                 # tag TRUE = registered host lambda
                            (lambda: mu(L.HEAD(L.TAIL(cell))(mu)(x)))       # impl(mu)(reduced operand)
                            (lambda: mu(mkapp(L.HEAD(L.TAIL(cell)))(x))))(  # compiled: mu(o : x)
                          L.SND(res)))
                      (lambda: L.BOT))(defs.FETCH(a)(store_fn()))
                on_seq = lambda l: mu(mkapp(L.HEAD(l))(       # metacomposition on the head
                    L.SEQ(L.CONS(fr)(L.CONS(x)(L.NIL)))))
                return fr(on_atom)(on_seq)(L.BOT)
            return L.IF(isapp(e))(lambda: reduce_app())(lambda: e)  # App reduces; a value is its meaning
        return step
    return L.Y(tau)                                          # mu = lfp tau


_MU = make_mu(defs.current)


def meaning(e):
    """mu e — reduce an FFP expression to its meaning (its normal form)."""
    return _MU(e)


def apply(f, x):
    """The FFP application (f : x), evaluated: mu(f : x)."""
    return _MU(mkapp(f)(x))
