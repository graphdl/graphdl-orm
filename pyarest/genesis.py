"""Install the base functions into DEFS (Backus §14.4.3, "installing the system
program"). Primitives are prim-wrapped host λ-terms — if-free, they let the
operand dispatch its own case; the named/derived functions are FFP objects
(Backus AST). This is the whole system in one place; `create` will be another
cell here, composed from these.
"""
from . import lam as L
from .objects import sq, app, bot, prim, sym, seq, seq2
from .defs import define

_T = sym("T")
_F = sym("F")
_lst = lambda o: o(lambda a: L.NIL)(lambda s: s)(lambda p: lambda q: L.NIL)(L.NIL)(lambda g: L.NIL)

# --- primitive λ-terms: the operand decides its own case (no `if`) -----------
define(sym("1"), prim(lambda x: x(lambda a: bot)(lambda s: L.HEAD(s))(lambda o: lambda r: bot)(bot)(lambda g: bot)))
define(sym("tl"), prim(lambda x: x(lambda a: bot)(lambda s: sq(L.TAIL(s)))(lambda o: lambda r: bot)(bot)(lambda g: bot)))
define(sym("id"), prim(lambda x: x))
define(sym("atom"), prim(lambda x: x(lambda a: _T)(lambda s: L.ISNIL(s)(_T)(_F))(lambda o: lambda r: _F)(_F)(lambda g: _F)))
define(sym("null"), prim(lambda x: x(lambda a: _F)(lambda s: L.ISNIL(s)(_T)(_F))(lambda o: lambda r: _F)(_F)(lambda g: _F)))
define(sym("apply"), prim(lambda y: y(lambda a: bot)(lambda s: app(L.HEAD(s))(L.HEAD(L.TAIL(s))))(lambda o: lambda r: bot)(bot)(lambda g: bot)))
define(sym("distr"), prim(lambda y: y(lambda a: bot)(lambda s: sq(L.MAP(lambda yi: seq2(yi)(L.HEAD(L.TAIL(s))))(_lst(L.HEAD(s)))))(lambda o: lambda r: bot)(bot)(lambda g: bot)))
# COMP: (ρ⟨COMP,f1..fn⟩):x = f1:(...(fn:x))  — insert (FOLDR) of apply, not a loop
define(sym("COMP"), prim(lambda a: a(lambda t: bot)(lambda s: L.FOLDR(lambda f: lambda acc: app(f)(acc))(L.HEAD(L.TAIL(s)))(L.TAIL(_lst(L.HEAD(s)))))(lambda o: lambda r: bot)(bot)(lambda g: bot)))
# ALPHA: (ρ⟨ALPHA,f⟩):x = ⟨f:x1,...,f:xn⟩  — apply-to-all (MAP), not a loop
define(sym("ALPHA"), prim(lambda a: a(lambda t: bot)(lambda s: sq(L.MAP(lambda xi: app(L.HEAD(L.TAIL(_lst(L.HEAD(s)))))(xi))(_lst(L.HEAD(L.TAIL(s))))))(lambda o: lambda r: bot)(bot)(lambda g: bot)))

# --- named / derived functions as FFP objects (Backus AST) -------------------
define(sym("2"), seq(sym("COMP"), sym("1"), sym("tl")))              # 1∘tl
define(sym("3"), seq(sym("COMP"), sym("1"), sym("tl"), sym("tl")))  # 1∘tl∘tl
define(sym("CONST"), seq(sym("COMP"), sym("2"), sym("1")))          # 2∘1
define(sym("CONS"), seq(sym("COMP"), seq(sym("ALPHA"), sym("apply")), sym("tl"), sym("distr")))  # α·apply∘tl∘distr
