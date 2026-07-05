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


AGG_MODEL = """Team is an entity type.
Player is an entity type.
Roster Size is a value type.
Player plays for Team.
Team has Roster Size iff Roster Size is the count of Player where Player \
plays for that Team.
Player 'p1' plays for Team 't1'.
Player 'p2' plays for Team 't1'.
Player 'p3' plays for Team 't2'.
"""


@pytest.mark.skipif(not polyglot.rust_available(), reason="no rust binary")
def test_rust_aggregate_fixpoint_matches_python():
    # the differential over the AGG PASS: the native carrier routes aggregate
    # rules through NEval too, so the cross-host check must cover them, not just
    # the closure the state-machine model exercises
    D, rep = forml.compile_model(AGG_MODEL)
    assert rep["unparsed"] == []
    heads = sorted({r[1] for r in system._pop_rows(D, "ruleDerives")
                    if len(r) >= 2})
    assert heads, "the model must carry an aggregate rule"
    D2 = system.run_rules(D)
    want = {h: {tuple(str(x) for x in r) for r in system._pop_rows(D2, h)}
            for h in heads}
    assert any(want.values())
    s = polyglot.RustSession()
    try:
        s.set_store(D)
        out = s._rpc({"op": "run_rules"})
        err = out.get("error", "") if isinstance(out, dict) else ""
        if "unknown op" in str(err):
            pytest.skip("the resident lacks run_rules")
        got = {}
        for h in heads:
            res = s._rpc({"op": "query", "fact_type": h})
            rows = (res.get("result", {}) or {}).get("rows", [])
            got[h] = {tuple(str(x) for x in r) for r in rows}
    finally:
        s.close()
    assert got == want                                        # t1 has 2, t2 has 1


CYCLIC_MODEL = """Node is an entity type.
Node links to Node.
Node reaches Node. *
* Node1 reaches Node2 iff Node1 links to Node2.
* Node1 reaches Node2 iff Node1 links to Node3 and Node3 reaches Node2.
Node 'a' links to Node 'b'.
Node 'b' links to Node 'c'.
"""


@pytest.mark.skipif(not polyglot.rust_available(), reason="no rust binary")
def test_rust_self_supporting_cyclic_fixpoint_matches_python():
    # the differential over the DRed sweep_cyclic pass: a fully-derived head
    # that reads itself (transitive closure) takes the empty-first recursive
    # form, and the native carrier routes it through NEval, so the cross-host
    # check must reach it (the fleet proved it on kernel; this pins it portably)
    D, rep = forml.compile_model(CYCLIC_MODEL)
    assert rep["unparsed"] == []
    heads = sorted({r[1] for r in system._pop_rows(D, "ruleDerives")
                    if len(r) >= 2})
    kinds = {r[0]: r[1] for r in system._pop_rows(D, "derivation")}
    assert any(kinds.get(h) == "fully-derived" for h in heads), \
        "the head must be fully-derived to exercise the sweep"
    D2 = system.run_rules(D)
    want = {h: {tuple(str(x) for x in r) for r in system._pop_rows(D2, h)}
            for h in heads}
    assert want["Node_reaches_Node"] == {("a", "b"), ("b", "c"), ("a", "c")}
    s = polyglot.RustSession()
    try:
        s.set_store(D)
        out = s._rpc({"op": "run_rules"})
        err = out.get("error", "") if isinstance(out, dict) else ""
        if "unknown op" in str(err):
            pytest.skip("the resident lacks run_rules")
        got = {}
        for h in heads:
            res = s._rpc({"op": "query", "fact_type": h})
            rows = (res.get("result", {}) or {}).get("rows", [])
            got[h] = {tuple(str(x) for x in r) for r in rows}
    finally:
        s.close()
    assert got == want                                        # the closure, both hosts
