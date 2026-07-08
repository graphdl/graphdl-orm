"""One-shot Registry verbs for the Rust resident's write delegation.

The resident keeps the read path hot over sidecars and spawns this script for
the verbs that need the compiler host: apply, retract, and compile. Each
invocation runs exactly one Registry verb through the same pipeline the
Python MCP server uses, prints one JSON receipt on stdout, and exits 0 on a
committed write (or a clean compile), 1 on a refusal, and 2 on a usage error.
The script self-registers the pyarest package from the python/ host directory
(the same bootstrap conftest.py performs), so it needs no install and no
particular working directory."""
import importlib.util
import json
import os
import sys

_ROOT = os.path.dirname(os.path.abspath(__file__))

if "pyarest" not in sys.modules:
    spec = importlib.util.spec_from_file_location(
        "pyarest", os.path.join(_ROOT, "python", "__init__.py"),
        submodule_search_locations=[os.path.join(_ROOT, "python")])
    mod = importlib.util.module_from_spec(spec)
    sys.modules["pyarest"] = mod
    spec.loader.exec_module(mod)

_USAGE = ("usage: cli.py <verb> --apps-dir <dir> <app> [args...]\n"
          "write verbs: compile <app> | apply <app> <fact_type> <row-json> |"
          " retract <app> <fact_type> <row-json>\n"
          "read verbs: get <app> <noun> <id> | schema <app> | sql <app>"
          " <statement> | explain <app> <id> | validate <app> | verify <app>"
          " | actions <app> <noun> <id> | synthesize <app> <id>\n"
          "desktop: show <app> [noun] — the store as a native window;"
          " controls resolve through DEFS, events apply facts\n")

# Each read verb names its Registry method and the arity of its trailing
# arguments; the CLI is a thin delegate, so outputs pass through as the
# method answers them.
_READS = {"get": 3, "schema": 1, "sql": 2, "explain": 2, "validate": 1,
          "verify": 1, "actions": 3, "synthesize": 2}


def main(argv):
    args = list(argv[1:])
    if len(args) >= 3 and args[1] == "--apps-dir":
        verb, apps_dir, rest = args[0], args[2], args[3:]
    else:
        sys.stderr.write(_USAGE)
        return 2
    import pyarest.prims  # noqa: F401
    from pyarest import apps
    reg = apps.Registry(apps_dir)
    if verb == "compile" and len(rest) == 1:
        out = reg.compile(rest[0])
        print(json.dumps(out, default=str))
        return 0
    if verb in ("apply", "retract") and len(rest) == 3:
        app, ft, row = rest[0], rest[1], tuple(json.loads(rest[2]))
        out = getattr(reg, verb)(app, ft, row)
        print(json.dumps(out, default=str))
        return 0 if out.get("committed") else 1
    if verb in _READS and len(rest) == _READS[verb]:
        try:
            out = getattr(reg, verb)(*rest)
        except Exception as e:
            # a read verb's failure on caller input (a bad SQL statement, an
            # unknown noun) is the CALLER'S error, answered as an envelope at
            # exit 0 so the resident relays it as a result; real crashes in
            # the write path keep their nonzero exits
            print(json.dumps({"error": f"{type(e).__name__}: {e}"},
                             default=str))
            return 0
        print(json.dumps(out, default=str))
        return 0
    if verb == "show" and len(rest) in (1, 2):
        from pyarest import showui
        showui.show(reg, rest[0], rest[1] if len(rest) > 1 else None)
        return 0
    if verb == "call" and len(rest) == 2:
        # the generic form: EVERY first-class verb through the one dispatch
        # (protocol.SESSION_VERBS / APP_VERBS) — the CLI is a binding like the
        # MCP servers, never a second verb table.
        #   cli.py call --apps-dir <dir> <verb> <json-args>
        from pyarest import mcp_server
        name, vargs = rest[0], json.loads(rest[1] or "{}")
        try:
            out = mcp_server._dispatch(reg, name, vargs)
        except Exception as e:
            print(json.dumps({"error": f"{type(e).__name__}: {e}"},
                             default=str))
            return 0
        print(json.dumps(out, default=str))
        return 0
    sys.stderr.write(_USAGE)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
