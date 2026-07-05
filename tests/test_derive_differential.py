"""Phase-two acceptance for the Rust derivation port: both hosts start from
the SAME underived base (compile_model without run_rules), derive to the
fixpoint, and every ruleDerives head compares row-for-row. Skips until the
resident grows the run_rules op (phase one of the port), and thereafter
certifies every later phase (semi-naive, aggregates, DRed, strata) against
the Python engine of record."""
import pytest

import pyarest.prims  # noqa: F401
from pyarest import forml, polyglot, system

MODEL = """Resource is an entity type.
State Machine is an entity type.
Status is a value type.
Task is an entity type.
Task Status is a value type.
State Machine is for Resource.
State Machine is currently in Status.
Resource is currently in Status.
Task has Task Status.

* Resource is currently in Status iff some State Machine is for that \
Resource and that State Machine is currently in that Status.

State Machine 'sm1' is for Resource 't1'.
State Machine 'sm1' is currently in Status 'in_progress'.
"""


@pytest.mark.skipif(not polyglot.rust_available(), reason="no rust binary")
def test_rust_fixpoint_matches_python_per_head():
    D, rep = forml.compile_model(MODEL)
    heads = sorted({r[1] for r in system._pop_rows(D, "ruleDerives")
                    if len(r) >= 2})
    assert heads, "the model must carry a derivation rule"
    D2 = system.run_rules(D)
    want = {h: {tuple(str(x) for x in r) for r in system._pop_rows(D2, h)}
            for h in heads}
    assert any(want.values()), "python must derive at least one row"
    s = polyglot.RustSession()
    try:
        s.set_store(D)
        out = s._rpc({"op": "run_rules"})
        err = out.get("error", "") if isinstance(out, dict) else ""
        if "unknown op" in str(err):
            pytest.skip("the resident lacks run_rules (phase one pending)")
        got = {}
        for h in heads:
            res = s._rpc({"op": "query", "fact_type": h})
            rows = (res.get("result", {}) or {}).get("rows", [])
            got[h] = {tuple(str(x) for x in r) for r in rows}
    finally:
        s.close()
    assert got == want
