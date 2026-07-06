"""The MCP binding end to end: a real subprocess speaking newline-delimited
JSON-RPC over stdio — initialize, tools/list, then the apps family and the read
tools against a real apps directory. The server is a platform binding; the engine
underneath is the same compile-to-lfp-and-snapshot the apps tests gate."""
import json
import os
import subprocess
import sys

import pyarest.prims  # noqa: F401


def _mk_app(root, name, text):
    d = os.path.join(root, name, "readings")
    os.makedirs(d)
    with open(os.path.join(d, "core.md"), "w", encoding="utf-8") as f:
        f.write(text)


def test_the_server_speaks_mcp_over_stdio(tmp_path):
    root = str(tmp_path)
    _mk_app(root, "tasks", "Task(.id) is an entity type.\nStatus is a value type.\n"
                           "Task has Status.\nTask 't1' has Status 'open'.\n")
    env = dict(os.environ)
    repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    proc = subprocess.Popen(
        [sys.executable, "-c",
         "import sys, importlib.util, os; root=sys.argv[1];"
         "spec=importlib.util.spec_from_file_location('pyarest',"
         " os.path.join(root,'python','__init__.py'),"
         " submodule_search_locations=[os.path.join(root,'python')]);"
         "m=importlib.util.module_from_spec(spec); sys.modules['pyarest']=m;"
         "spec.loader.exec_module(m);"
         "from pyarest import mcp_server; mcp_server.serve(sys.argv[2])",
         repo, root],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, text=True,
        encoding="utf-8")

    def rpc(mid, method, params=None):
        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": mid,
                                     "method": method,
                                     "params": params or {}}) + "\n")
        proc.stdin.flush()
        return json.loads(proc.stdout.readline())

    try:
        init = rpc(1, "initialize", {"protocolVersion": "2024-11-05"})
        assert init["result"]["serverInfo"]["name"] == "pyarest"
        tools = rpc(2, "tools/list")
        names = {t["name"] for t in tools["result"]["tools"]}
        assert {"orient", "apps_use", "apps_compile", "query", "sql"} <= names
        rpc(3, "tools/call", {"name": "apps_use", "arguments": {"name": "tasks"}})
        comp = rpc(4, "tools/call", {"name": "apps_compile",
                                     "arguments": {"name": "tasks"}})
        body = json.loads(comp["result"]["content"][0]["text"])
        assert body["unparsed"] == []
        q = rpc(5, "tools/call", {"name": "query",
                                  "arguments": {"fact_type": "Task_has_Status"}})
        rows = json.loads(q["result"]["content"][0]["text"])["rows"]
        assert rows == [["t1", "open"]]
        bad = rpc(6, "tools/call", {"name": "query",
                                    "arguments": {"fact_type": "Nope"}})
        assert json.loads(bad["result"]["content"][0]["text"])["rows"] == []
    finally:
        proc.stdin.close()
        proc.wait(timeout=15)


def test_every_engine_verb_is_first_class(tmp_path):
    """Samuel, 2026-07-06: all verbs are FIRST-CLASS on the engine's verb
    surface — any binding (MCP, CLI, REST) advertises exactly the table it
    dispatches. The Registry-implemented verbs are all exposed, and the
    session serves engine_version, app status/create, the live additive
    compile (readings stay the source of truth), and the authoring dry-run
    (propose — classify and diagnose, never persist)."""
    from pyarest import apps, mcp_server
    tool_names = {t["name"] for t in mcp_server.TOOLS}
    verb_names = set(mcp_server.SESSION_VERBS) | set(mcp_server.APP_VERBS)
    assert tool_names == verb_names                           # advertise == dispatch
    assert {"validate", "verify", "actions", "synthesize", "explain",
            "engine_version", "apps_status", "apps_create", "compile",
            "propose"} <= verb_names
    root = str(tmp_path)
    _mk_app(root, "flow", "Task(.id) is an entity type.\nStatus is a value type.\n"
                          "Task has Status.\nTask 't1' has Status 'open'.\n")
    reg = apps.Registry(root)
    reg.compile("flow")
    reg.use("flow")
    out = mcp_server._dispatch(reg, "actions", {"noun": "Task", "id": "t1"})
    assert out["noun"] == "Task"
    ver = mcp_server._dispatch(reg, "engine_version", {})
    assert ver["version"] == "0.9.0" and ver["engine"] == "pyarest"
    st = mcp_server._dispatch(reg, "apps_status", {"name": "flow"})
    assert st["name"] == "flow" and st["compiled"] is True and st["stale"] is False
    # propose: the authoring dry-run — classify + diagnose, nothing lands
    prop = mcp_server._dispatch(reg, "propose", {
        "text": "Task is blocked by Task.\n"})
    assert "Task_is_blocked_by_Task" in prop["would_declare"]
    assert reg.query("flow", "Task_is_blocked_by_Task") == []
    # compile: live ADDITIVE readings — durable (they join readings/, so a
    # from-scratch rebuild keeps them; the source of truth stays the readings)
    add = mcp_server._dispatch(reg, "compile", {
        "text": "Task 't2' has Status 'open'.\n"})
    assert add["unparsed"] == []
    assert ["t2", "open"] in [list(r) for r in reg.query("flow", "Task_has_Status")]
    reg.compile("flow")                                       # rebuild keeps it
    assert ["t2", "open"] in [list(r) for r in reg.query("flow", "Task_has_Status")]
    # apps_create: the new-app skeleton
    made = mcp_server._dispatch(reg, "apps_create", {
        "name": "fresh", "text": "Note is an entity type.\n"})
    assert made["created"] == "fresh"
    assert os.path.exists(os.path.join(root, "fresh", "readings", "core.md"))
    # ask: with a plan the projection executes; without one, the caller gets
    # the model surface to complete the plan (no LLM in the engine)
    got = mcp_server._dispatch(reg, "ask", {
        "question": "which tasks are open?",
        "plan": {"fact_type": "Task_has_Status", "filter": {"Status": "open"}}})
    assert {tuple(r) for r in got["rows"]} == {("t1", "open"), ("t2", "open")}
    nop = mcp_server._dispatch(reg, "ask", {"question": "which tasks are open?"})
    assert nop["needs_plan"] and "model" in nop
    # apps_check: the registry-wide sweep
    chk = mcp_server._dispatch(reg, "apps_check", {})
    assert chk["summary"].get("ready", 0) >= 1
