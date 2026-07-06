"""pyarest — the AREST engine on Backus's FP-on-lambda, in seven files
shaped like the sibling hosts (rust one, csharp four, java five).

`kernel` — the whole evaluator stack: the λ substrate (Scott encodings,
Church booleans, Y, the ⊥ discipline), the definition store and enumerable
boundary, the native delta fast-path (held observationally equal by the
differential oracle), mu = Y(tau) as the ground truth, and the Backus base.
`canon` — the intersection-source vocabulary, the repo layout, and Codd's
θ₁ bindings (binds, never authors). `compiler` — the metamodel M and the
FORML compiler (Stage-1 the bootstrap kernel, dispatching exactly its five
measured kinds; the grammar file is the parser). `engine` — cells and the
store walk, the violation expressions, the create pipeline, and derive to
the least fixed point with the joint strata. `protocol` — the event log and
freeze/thaw with sealing at rest, the RMAP projection, the swap tool,
federation, the apps registry, and the MCP binding. `tools` — the two-level
optimizer and the Rust kernel seam.

The old module names (pyarest.lam, .defs, .delta, .reduce, .prims, .theta,
.paths, .machine, .meta, .forml, .ast, .constraints, .system, .persist,
.ddl, .migrate, .federate, .apps, .mcp_server, .optimize, .polyglot) all
resolve through the alias table below, so call sites keep their idioms.

THE SHARED SOURCE IS POLYGLOT. shared/ holds only sources every host
consumes as written: the readings (FORML), the canon (theta, constraints,
ast, system), and the cross-host case table (scenarios). A stratum retires
host-side as its content lands in shared/; per-host optimizations are DEFS
registrations, never forks of the source. The root conftest constructs the
package for the checkout; installs map it in pyproject."""

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
from . import engine as _engine
for _n in ("ast", "constraints", "system"):
    _sys.modules[__name__ + "." + _n] = _engine
ast = constraints = system = _engine
from . import compiler as _compiler
for _n in ("meta", "forml"):
    _sys.modules[__name__ + "." + _n] = _compiler
meta = forml = _compiler
from . import protocol as _protocol
for _n in ("persist", "ddl", "migrate", "federate", "apps", "mcp_server"):
    _sys.modules[__name__ + "." + _n] = _protocol
persist = ddl = migrate = federate = apps = mcp_server = _protocol
from . import tools as _tools
for _n in ("optimize", "polyglot"):
    _sys.modules[__name__ + "." + _n] = _tools
optimize = polyglot = _tools
