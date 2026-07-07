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


def test_rust_derive_reconciles_absorbed_heads_to_the_columns(tmp_path):
    """view == reassembly for DERIVED heads, cross-host: after an apply whose
    ripple derives an ABSORBED head, the entity's table row carries the
    derived column in BOTH engines — python's run_rules reconciles, and the
    resident's bounded native derive must reconcile identically or the
    stores diverge row-for-row."""
    import json
    import os
    import shutil
    import subprocess
    import pytest
    from pyarest import apps as _apps, canon as _canon, system
    exe = _canon.rust_bin("arestlam")
    if not os.path.exists(exe):
        pytest.skip("rust kernel not built")
    MODEL = """Person(.Name) is an entity type.
Room is a value type.
Person was in Room.
Each Person was in at most one Room.
Person1 is placed if Person1 was in some Room1.
"""
    root = str(tmp_path / "apps")
    d = os.path.join(root, "flow", "readings")
    os.makedirs(d)
    with open(os.path.join(d, "app.md"), "w", encoding="utf-8") as f:
        f.write(MODEL)
    reg = _apps.Registry(root, cache_dir=str(tmp_path / "fz"))
    reg.compile("flow")
    # snapshot the compiled app for the resident BEFORE the python apply
    root2 = str(tmp_path / "apps2")
    shutil.copytree(root, root2)
    # PYTHON: apply + ripple
    reg.apply("flow", "Person_was_in_Room", ("Adler", "library"))
    D = reg._load("flow")
    py_row = next(tuple(c[2]) for c in
                  __import__("pyarest").lam.from_lam(D)
                  if isinstance(c, tuple) and len(c) >= 3
                  and c[1] == "Person:Adler")
    assert "T" in py_row                                      # the derived column landed
    # RUST: the same apply natively (delegation disabled), then read the row
    proc = subprocess.Popen(
        [exe, "--mcp", "--apps-dir", root2, "--python", "no-such-interpreter"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        encoding="utf-8")

    def rpc(mid, method, params=None):
        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": mid,
                                     "method": method,
                                     "params": params or {}}) + "\n")
        proc.stdin.flush()
        return json.loads(proc.stdout.readline())

    try:
        rpc(1, "initialize", {"protocolVersion": "2024-11-05"})
        rpc(2, "tools/call", {"name": "apps_use", "arguments": {"name": "flow"}})
        ap = rpc(3, "tools/call", {"name": "apply", "arguments": {
            "app": "flow", "fact_type": "Person_was_in_Room",
            "fact": ["Adler", "library"]}})
        assert json.loads(ap["result"]["content"][0]["text"])["committed"] is True
    finally:
        proc.kill()
    # the resident persisted its store to the sidecar: read the row there
    side = json.load(open(os.path.join(root2, "flow", "flow.store.json"),
                          encoding="utf-8"))
    rust_row = next((tuple(c[2]) for c in side["d"]
                     if isinstance(c, list) and len(c) >= 3
                     and c[0] == "CELL" and c[1] == "Person:Adler"), None)
    assert rust_row == py_row
