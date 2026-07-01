"""The reducer: ρ (reflection) + apply, with Backus's metacomposition rule."""
from .objects import Atom, Seq, seq, BOTTOM, is_seq, is_bottom, PHI
from .defs import DEFS


def _bottom_fn(_x):
    return BOTTOM


def rho(f):
    """The representation function ρ (Backus §13.3.2): the function f denotes."""
    if is_bottom(f):
        return _bottom_fn
    if is_seq(f):
        if len(f.items) == 0:          # φ as operator — no controlling op
            return _bottom_fn
        head_op = f.items[0]
        # metacomposition: (ρ⟨x1..xn⟩):y = (ρx1):⟨⟨x1..xn⟩, y⟩
        return lambda y: apply(head_op, seq(f, y))
    if isinstance(f, Atom):
        d = DEFS.get(f.value)
        if d is None:
            return _bottom_fn
        if d.origin == "registered":
            return d.impl              # host callable
        return rho(d.impl)             # Def n ≡ ρc  (compiled body)
    return _bottom_fn


def apply(f, x):
    """Evaluate the FFP application (f : x) = μ(ρf : x) (Backus §13.3.3, prop. 7)."""
    return rho(f)(x)
