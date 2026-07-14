"""host_py — the rebuild's Python μ-evaluator seed (SPEC §9; PLAN Days 2–3).

Day-2 scope: `kernel` (μ over the H1 forms, the definition store and the
enumerable boundary, the Backus base) and `canon` (the polyglot arest.canon
binding at the repo root, Codd's θ₁ — binds, never authors), plus `tromp`
(the G1 ρ-fidelity litmus checker, lazily importing the two). The command
stages, the one gate, and the transports arrive Day 3 against SPEC 2.x, §12.
"""

from . import kernel
import sys as _sys
for _n in ("lam", "defs", "delta", "reduce", "prims"):
    # the quarry's alias table: the old module names all resolve to the kernel
    _sys.modules[__name__ + "." + _n] = kernel
lam = defs = delta = reduce = prims = kernel
from . import canon
