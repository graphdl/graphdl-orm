"""pyarest — the AREST engine on Backus's FP-on-lambda.

Rebuild in progress (the way the papers say): a raw-lambda kernel (lam), the DEFS
store (defs), the meaning function mu = Y(tau) (reduce), and Backus's base as
controlling operators (prims). The upper layers — Codd theta1, the self-capturing
metamodel M, the constraint violation expressions, RMAP/CSDP, verbalize, create —
are being re-authored on this kernel as FFP objects / FORML readings; the native
modules (objects, theta, orm, system, ast, forml, machine, pop, forms) are retired
and not imported until re-seated on the kernel.
"""
from . import lam, defs, reduce, prims                       # the lambda kernel + Backus base
from .lam import ATOM, SEQ, BOT, PHI, to_lam, from_lam, atom
from .reduce import apply, meaning, mkapp
