#!/usr/bin/env python3
"""Receipt parity for the native `verify` verb vs the Python delegate.

`verify` (protocol.Registry.verify) audits the compiled store: for each derivation
head that MUST reproduce (the destructive sweep/dred/aggwhole passes plus the
derivation-OWNED keyed heads), it checks the head's stored rows equal the rows its
rules recompute over D — a mismatch means a tampered .db or a store saved before the
rules changed. It was Python-only: with no cli.py resolvable the verb hard-failed
("no cli.py found") — unacceptable in a Rust-configured environment with no Python.

The Rust resident now carries a native `verify` (main.rs native_verify), reusing the
native classify_heads twin and the same reduce_over the run_rules fixpoint applies a
rule through — no Python, no cli.py. It is wired as the Python-absent fallback exactly
like apps_compile: native when AREST_NATIVE_VERIFY is set OR apps.cli is None; Python
present + no flag stays the delegate reference this harness certifies the native path
against.

This harness compiles each app, then verifies it BOTH ways over the (byte-equal)
store and compares the receipts head by head:
  * Python:  Registry.compile(name); Registry.verify(name)          -> {"app","checks"}
  * Native:  resident apps_compile -> apps_use -> verify (flagged)  -> {"app","checks"}
each check is {head, stored, recomputed, match}; parity means identical head sets and
identical (stored, recomputed, match) per head. Companion to apps_compile_parity.py
(compile receipt = the store) and twin_equality.py (the prim-twin equality axis); this
closes the verb seam's Python dependency for verify.

Usage:  python engine/tools/verify_parity.py [app1 app2 ...]
        APPS_DIR=/path/to/apps  AREST_BIN=/path/to/arest.exe   (optional)
Exit 0 iff every app's native and Python verify receipts agree.
"""
import os
import sys
import json
import shutil
import tempfile
import subprocess
import importlib.util

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # engine/
APPS_DIR = os.environ.get("APPS_DIR", os.path.join(os.path.dirname(_ROOT), "..", "apps"))
BIN = os.environ.get("AREST_BIN", os.path.join(_ROOT, "rust", "target", "release", "arest.exe"))

DEFAULT_SAMPLE = ["listings-vdp", "spd-guardian", "message-vetting", "charge-dispute-service"]


def _load_pyarest():
    spec = importlib.util.spec_from_file_location(
        "pyarest", os.path.join(_ROOT, "python", "__init__.py"),
        submodule_search_locations=[os.path.join(_ROOT, "python")])
    m = importlib.util.module_from_spec(spec)
    sys.modules["pyarest"] = m
    spec.loader.exec_module(m)
    return m


def _native_verify(name, scratch):
    """Resident session: native compile -> load -> native verify. Returns the
    verify receipt (id 3) or (None, diagnostic)."""
    env = dict(os.environ)
    env["AREST_NATIVE_COMPILE"] = "1"
    env["AREST_NATIVE_VERIFY"] = "1"

    def call(i, tool, argd):
        return json.dumps({"jsonrpc": "2.0", "id": i, "method": "tools/call",
                           "params": {"name": tool, "arguments": argd}}) + "\n"
    stdin = (call(1, "apps_compile", {"app": name})
             + call(2, "apps_use", {"name": name})
             + call(3, "verify", {}))
    p = subprocess.run([BIN, "--mcp", "--apps-dir", scratch], input=stdin,
                       capture_output=True, text=True, timeout=600, env=env)
    receipt = None
    for line in p.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except Exception:
            continue
        if obj.get("id") == 3:
            res = obj.get("result", {})
            txt = None
            if isinstance(res, dict) and "content" in res:
                for c in res["content"]:
                    if c.get("type") == "text":
                        txt = c.get("text")
            receipt = json.loads(txt) if txt else res
    if receipt is None:
        return None, (p.stdout[-400:] + " | STDERR " + p.stderr[-400:])
    return receipt, None


def _bag(receipt):
    return {c["head"]: (c["stored"], c["recomputed"], c["match"])
            for c in receipt.get("checks", [])}


def verify_parity(names):
    _load_pyarest()
    import pyarest.prims  # noqa: F401
    from pyarest import apps as A
    base = A.default_base()
    ok_all = True
    for name in names:
        src = os.path.join(APPS_DIR, name, "readings")
        if not os.path.isdir(src):
            print(f"{name:24} SKIP (no readings/)")
            continue
        scratch = tempfile.mkdtemp(prefix=f"vp_{name}_")
        try:
            rd = os.path.join(scratch, name, "readings")
            os.makedirs(rd)
            for f in os.listdir(src):
                if f.endswith(".md"):
                    shutil.copy(os.path.join(src, f), os.path.join(rd, f))
            A.Registry(scratch, base_dir=base).compile(name)
            py = A.Registry(scratch, base_dir=base).verify(name)
            nat, err = _native_verify(name, scratch)
            if nat is None:
                print(f"{name:24} NATIVE ERR: {err}")
                ok_all = False
                continue
            pb, nb = _bag(py), _bag(nat)
            only_py = sorted(set(pb) - set(nb))
            only_nat = sorted(set(nb) - set(pb))
            diff = [h for h in (set(pb) & set(nb)) if pb[h] != nb[h]]
            ok = not only_py and not only_nat and not diff
            ok_all = ok_all and ok
            if ok:
                print(f"{name:24} PARITY  ({len(pb)} checks)")
            else:
                print(f"{name:24} DIVERGE only_py={only_py[:3]} only_nat={only_nat[:3]} "
                      f"diff={[(h, pb[h], nb[h]) for h in diff[:3]]}")
        finally:
            shutil.rmtree(scratch, ignore_errors=True)
    print("\nVERDICT:", "VERIFY PARITY" if ok_all else "DIVERGENCE — investigate above")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(verify_parity(sys.argv[1:] or DEFAULT_SAMPLE))
