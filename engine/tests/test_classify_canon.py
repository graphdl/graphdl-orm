"""The scheduler classification IS canon (the doctrine directive,
2026-07-08: "all functionality available in a performant override must be
defined in the shared lambda base"): system:classify_heads over the six
fetched M-pops ⟨ruleAgg, ruleDerives, derivation, spans, constraint,
ruleReads⟩ answers the passHeads rows ⟨pass, head⟩, and the python
_classify_heads run_rules consumes is the certified-equal performant
override, twinned here on every run. Populations are sets (order is
presentation, owned by the materializing host), so the twin compares
per-pass row SETS."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest import ast, forml
from pyarest.lam import atom as A, from_lam
from pyarest.reduce import apply
from pyarest.engine import _classify_heads

MODEL = """Task(.id) is an entity type.
Peer(.id) is an entity type.
Cost is a value type.
Rank is a value type.
Cost Total is a value type.
Cost Tally is a value type.
Peer serves Task.
Peer has Cost.
Peer has Rank.
Task blocks Task.
Task has Cost. **
Task has Rank. **
Each Task has at most one Rank.
Task is reachable. **
Task is urgent.
Task has Cost Total. **
Task has Cost Tally.

* Task has Cost iff some Peer serves that Task and that Peer has Cost.
* Task has Rank iff some Peer serves that Task and that Peer has Rank.
* Task is reachable iff the Task blocks some Task1 and Task1 has Cost.
* Task is reachable iff the Task blocks some Task1 and Task1 is reachable.
* Task is urgent iff the Task blocks some Task1 and Task1 has Cost.
* Task1 has Cost Total iff Cost Total is the sum of Cost1 where Task1 has Cost1.
* Task1 has Cost Tally iff Cost Tally is the count of Cost1 where Task1 has Cost1.
"""

_POPS = ("ruleAgg", "ruleDerives", "derivation", "spans",
         "constraint", "ruleReads")


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _canon_rows(D):
    pops = _S(*[apply(ast.FetchPop(n), D) for n in _POPS])
    out = from_lam(apply(A("system:classify_heads"), pops))
    assert isinstance(out, tuple), f"the def must answer rows, got {out!r}"
    return {tuple(r) for r in out if isinstance(r, tuple)}


def test_the_canonical_classification_twins_the_host_override():
    D, _rep = forml.compile_model(MODEL)
    classes = _classify_heads(D)
    want = {(p, h)
            for p in ("agg", "keyed", "sweep", "dred", "aggwhole")
            for h in classes[p]}
    assert _canon_rows(D) == want


def test_the_twin_holds_on_an_empty_rule_surface():
    # a model with no rules at all: every pass is empty, the def answers
    # the empty population (PHI), not bottom
    D, _rep = forml.compile_model(
        "Widget is an entity type.\nColor is a value type.\n"
        "Widget has Color.\n")
    assert _canon_rows(D) == set()
