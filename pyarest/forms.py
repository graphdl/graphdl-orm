"""Derived functions as FFP objects (Backus AST) in DEFS. Higher selectors are
compositions of the primitives 1 and tl — n ≡ 1∘tl^(n-1) — not host code.
"""
from .objects import Atom, Seq
from .defs import define

_COMP = Atom("COMP")
_1 = Atom("1")
_TL = Atom("tl")

_selector = lambda n: Seq((_COMP, _1) + tuple(_TL for _ in range(n - 1)))

tuple(map(lambda n: define(str(n), _selector(n)), (2, 3, 4, 5, 6)))

# concatenation X ++ Y ≡ (/apndl) ∘ apndr — a defined FFP object, not a primitive
_INSERT = Atom("INSERT")
_APNDL = Atom("apndl")
_APNDR = Atom("apndr")
define("cat", Seq((_COMP, Seq((_INSERT, _APNDL)), _APNDR)))
