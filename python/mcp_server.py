"""The MCP binding (the swap contract, part 2): the old engine's daily-driver
surface served over the Model Context Protocol's stdio transport, newline-delimited
JSON-RPC 2.0, in the stdlib only — a platform binding in the paper's sense (a
server registers its functions; the engine does not change). It carries the orientation and apps family, the read tools (query, sql),
and the write path: apply (eq. create with the receipt) and context (the
last mutation receipt). retract rides the same write path; the tutor/induce family follows.

Run: python -m pyarest.mcp_server <apps_dir>   (or PYAREST_APPS in the env).
"""
import json
import os
import sys

from . import apps

TOOLS = [
    {"name": "orient",
     "description": "Apps inventory + the active app, one envelope.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "apps_list",
     "description": "Every app under the apps directory (readings/ + .db).",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "apps_current",
     "description": "The active app name, from the persistent marker.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "apps_use",
     "description": "Switch the active app (persists the marker).",
     "inputSchema": {"type": "object", "properties": {
         "name": {"type": "string"}}, "required": ["name"]}},
    {"name": "apps_compile",
     "description": "Compile an app's readings to the lfp and snapshot its .db. "
                    "A from-scratch rebuild by design: supersession is correct.",
     "inputSchema": {"type": "object", "properties": {
         "name": {"type": "string"}}, "required": ["name"]}},
    {"name": "query",
     "description": "A fact type's population from the app's snapshot.",
     "inputSchema": {"type": "object", "properties": {
         "fact_type": {"type": "string"},
         "app": {"type": "string"}}, "required": ["fact_type"]}},
    {"name": "sql",
     "description": "Read-only SQL over the app's snapshot database.",
     "inputSchema": {"type": "object", "properties": {
         "statement": {"type": "string"},
         "app": {"type": "string"}}, "required": ["statement"]}},
    {"name": "apply",
     "description": "Create one fact (eq. create): validate, commit iff no "
                    "alethic violation, log, snapshot. Answers the receipt.",
     "inputSchema": {"type": "object", "properties": {
         "fact_type": {"type": "string"},
         "fact": {"type": "array", "items": {"type": "string"}},
         "app": {"type": "string"}}, "required": ["fact_type", "fact"]}},
    {"name": "retract",
     "description": "Logical deletion, validated: the shrunk population must "
                    "satisfy the schema; commits as a log entry + rebuild.",
     "inputSchema": {"type": "object", "properties": {
         "fact_type": {"type": "string"},
         "fact": {"type": "array", "items": {"type": "string"}},
         "app": {"type": "string"}}, "required": ["fact_type", "fact"]}},
    {"name": "context",
     "description": "The last mutation receipt (committed, violations).",
     "inputSchema": {"type": "object", "properties": {}}},
]


def _dispatch(reg, name, args):
    if name == "orient":
        return reg.orient()
    if name == "apps_list":
        return reg.list()
    if name == "apps_current":
        return {"current": reg.current()}
    if name == "apps_use":
        return {"active_app": reg.use(args["name"])}
    if name == "apps_compile":
        return reg.compile(args["name"])
    if name == "context":
        return reg.last_receipt or {"note": "no mutation this session"}
    app = args.get("app") or reg.current()
    if not app:
        raise ValueError("no app given and no active app set (apps_use first)")
    if name == "query":
        return {"app": app, "fact_type": args["fact_type"],
                "rows": reg.query(app, args["fact_type"])}
    if name == "sql":
        return {"app": app, "rows": reg.sql(app, args["statement"])}
    if name == "apply":
        return reg.apply(app, args["fact_type"], args["fact"])
    if name == "retract":
        return reg.retract(app, args["fact_type"], args["fact"])
    raise ValueError(f"unknown tool {name!r}")


def serve(apps_dir, stdin=None, stdout=None):
    stdin = stdin or sys.stdin
    stdout = stdout or sys.stdout
    reg = apps.Registry(apps_dir)
    for line in stdin:
        line = line.strip()
        if not line:
            continue
        msg = json.loads(line)
        mid = msg.get("id")
        method = msg.get("method")
        if method == "initialize":
            result = {"protocolVersion": msg["params"].get("protocolVersion",
                                                           "2024-11-05"),
                      "capabilities": {"tools": {}},
                      "serverInfo": {"name": "pyarest", "version": "0.0.1"}}
        elif method == "tools/list":
            result = {"tools": TOOLS}
        elif method == "tools/call":
            p = msg.get("params", {})
            try:
                out = _dispatch(reg, p.get("name"), p.get("arguments") or {})
                result = {"content": [{"type": "text",
                                       "text": json.dumps(out, default=str)}]}
            except Exception as e:                             # tool errors are results
                result = {"content": [{"type": "text", "text": str(e)}],
                          "isError": True}
        elif mid is None:
            continue                                           # notification: no reply
        else:
            stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid,
                                     "error": {"code": -32601,
                                               "message": f"unknown {method}"}}) + "\n")
            stdout.flush()
            continue
        if mid is not None:
            stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid,
                                     "result": result}) + "\n")
            stdout.flush()


if __name__ == "__main__":
    serve(sys.argv[1] if len(sys.argv) > 1 else os.environ.get("PYAREST_APPS", "."))
