"""Combining forms as controlling-operator primitives (Backus §13.3.2): COMP, CONS, CONST."""
from .objects import Atom, Seq, seq, BOTTOM, is_seq
from .defs import DEFS
from .reduce import apply

COMP = Atom("COMP")
CONS = Atom("CONS")
CONST = Atom("CONST")


def _p_const(arg):
    # arg = ⟨⟨CONST, x⟩, y⟩  →  x   (Backus: pCONST ≡ 2∘1)
    if not is_seq(arg) or len(arg.items) != 2:
        return BOTTOM
    whole = arg.items[0]
    if not is_seq(whole) or len(whole.items) < 2:
        return BOTTOM
    return whole.items[1]


def _p_cons(arg):
    # arg = ⟨⟨CONS, f1..fn⟩, x⟩  →  ⟨f1:x, ..., fn:x⟩
    if not is_seq(arg) or len(arg.items) != 2:
        return BOTTOM
    whole, x = arg.items[0], arg.items[1]
    fs = whole.items[1:]
    return seq(*[apply(f, x) for f in fs])


def _p_comp(arg):
    # arg = ⟨⟨COMP, f1..fn⟩, x⟩  →  f1:(f2:(...(fn:x)))
    if not is_seq(arg) or len(arg.items) != 2:
        return BOTTOM
    whole, x = arg.items[0], arg.items[1]
    acc = x
    for f in reversed(whole.items[1:]):
        acc = apply(f, acc)
    return acc


def register_forms(defs=DEFS):
    defs.register("CONST", _p_const)
    defs.register("CONS", _p_cons)
    defs.register("COMP", _p_comp)


register_forms()
