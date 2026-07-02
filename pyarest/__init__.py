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
Stage 2/3, per `arest/readings/forml2-grammar.md`).
"""
from . import lam, defs, reduce, prims                       # the lambda kernel + Backus base
from .lam import ATOM, SEQ, BOT, PHI, to_lam, from_lam, atom
from .reduce import apply, meaning, mkapp
