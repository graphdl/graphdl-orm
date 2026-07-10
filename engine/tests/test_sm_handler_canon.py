"""#18: the six pure state-machine reading handlers, canonized. Each host _h_sm_* delegates
to the canonical system:sm_rows over ⟨verb, head, g0, g1⟩; the canon twin system:h_sm_*
builds that four-tuple from ⟨groups, known, mod⟩ (constant verb/head + the two group
selectors) and wraps ⟨rows, phi⟩. This certifies host == canon (the #18 doctrine: the host
stays native for compile speed, the lambda is the meaning). Trigger/guard are excluded —
their second group needs known-context reading->ft resolution (the Stage-1 boundary)."""
import pyarest.prims  # noqa: F401
from pyarest import canon, compiler
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

canon.load_all()

HANDLERS = [
    ("system:h_sm_def", compiler._h_sm_def),
    ("system:h_sm_initial", compiler._h_sm_initial),
    ("system:h_sm_from", compiler._h_sm_from),
    ("system:h_sm_to", compiler._h_sm_to),
    ("system:h_sm_emit", compiler._h_sm_emit),
    ("system:h_sm_moore", compiler._h_sm_moore),
]

SAMPLES = [("Placed", "Order Lifecycle"), ("A", "B"),
           ("shipped", "X Machine"), ("s1", "s2")]


def _canon(name, groups):
    r = from_lam(R(A(name), to_lam((tuple(groups), (), ""))))
    asserts = [(x[0], tuple(x[1])) for x in r[0]]
    objs = list(r[1]) if len(r) > 1 else []
    return asserts, objs


def _host(fn, groups):
    a, o = fn(groups, None, None)
    return [(c, tuple(row)) for c, row in a], list(o)


def test_sm_handlers_twin_host():
    for name, fn in HANDLERS:
        for g in SAMPLES:
            assert _canon(name, g) == _host(fn, g), (name, g)


def test_sm_handlers_emit_rows():
    # a concrete shape check: def emits smDef + the M fact-type row, no objs
    a, o = _canon("system:h_sm_def", ("Order Lifecycle", "Order"))
    assert o == []
    cells = {c for c, _ in a}
    assert "smDef" in cells and "State_Machine_Definition_is_for_Noun" in cells
