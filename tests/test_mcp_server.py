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
