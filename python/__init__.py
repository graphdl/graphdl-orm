"""pyarest — the AREST engine on Backus's FP-on-lambda.

The kernel (the trusted base): `lam` — the λ substrate (Scott-encoded objects and lists,
Church booleans/pairs, Y-combinator recursion, the ⊥ discipline); `defs` — the definition
store, the enumerable boundary (registered host impls), and the per-step DEFS-in-D binding;
`reduce` — mu = Y(tau) with metacomposition as the only mechanism (the ground truth);
`delta` — the native fast-path, held observationally equal by the differential oracle
(tests/test_oracle.py); `prims` — the Backus base (§11.2.3–.4) registered into DEFS.

Authored above it as FFP objects (no raw λ, spec D4): `theta` — Codd θ₁; `constraints` —
the violation expressions, cell-local and cross-cell scoped; `system` — the create pipeline
stages, derive = lfp F_S, HATEOAS links, state machines read off M; `ast` — cells, ↑/↓,
the AST transition, eq. sys routing, DefineIn (DEFS into D); `machine` — the one fold
runner; `meta` — the metamodel M (vignette; the full self-capturing M is Phase 3);
`forml` — the FORML 2 seed compiler (Stage 1; superseded by grammar-as-readings in
Stage 2/3, per `shared/forml2-grammar.md`, vendored).

THE SHARED SOURCE IS POLYGLOT. shared/ holds only sources every host consumes as
written: the readings (FORML) and the canon (canonical definitions as carrier-free
object trees, shared/canon/). This directory (python/) is the Python host: the
lambda platform (lam, reduce, prims, defs), the boundary and bindings, and the
canonical-stratum AUTHORING TOOLCHAIN (theta, constraints, system, ast, meta —
authored in the Backus base, no host logic) whose job is to EMIT canon; a module
retires host-side as its content lands in shared/. Per-host optimizations (delta,
FAST, the native carrier) are DEFS registrations, never forks of the source. The
root conftest constructs the package for the checkout; installs map it in
pyproject."""

from . import kernel                                        # the whole evaluator stack, one file
import sys as _sys
for _n in ("lam", "defs", "delta", "reduce", "prims"):
    # the old module names stay importable (pyarest.lam and kin all resolve
    # to the kernel), so call sites and tests keep their idioms unchanged
    _sys.modules[__name__ + "." + _n] = kernel
lam = defs = delta = reduce = prims = kernel
from . import canon
canon.load_all()                                             # the INTERSECTION SOURCE, at boot:
                                                             # canonical names resolve everywhere
from .kernel import ATOM, SEQ, BOT, PHI, to_lam, from_lam, atom
from .kernel import apply, meaning, mkapp
