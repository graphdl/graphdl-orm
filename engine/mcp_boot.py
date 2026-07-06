"""The MCP stdio launcher for the 0.9.0 engine — no install required: the
package loads from this file's sibling python/ directory and serve() speaks
newline-delimited JSON-RPC over stdio. Usage:

    python -u engine/mcp_boot.py <apps_dir>     (or PYAREST_APPS in the env)

The verb surface is first-class (protocol.SESSION_VERBS / APP_VERBS); this
binding advertises exactly what it dispatches."""
import importlib.util
import os
import sys

_ROOT = os.path.dirname(os.path.abspath(__file__))
if "pyarest" not in sys.modules:
    _spec = importlib.util.spec_from_file_location(
        "pyarest", os.path.join(_ROOT, "python", "__init__.py"),
        submodule_search_locations=[os.path.join(_ROOT, "python")])
    _mod = importlib.util.module_from_spec(_spec)
    sys.modules["pyarest"] = _mod
    _spec.loader.exec_module(_mod)
import pyarest.prims  # noqa: E402,F401
from pyarest import mcp_server  # noqa: E402

if __name__ == "__main__":
    apps_dir = (sys.argv[1] if len(sys.argv) > 1
                else os.environ.get("PYAREST_APPS"))
    if not apps_dir:
        sys.stderr.write("usage: python mcp_boot.py <apps_dir>\n")
        sys.exit(2)
    mcp_server.serve(apps_dir)
