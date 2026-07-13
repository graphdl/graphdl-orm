#!/usr/bin/env python3
"""Receipt parity for the native `validate` verb vs the Python delegate.

`validate` (protocol.Registry.validate) checks the compiled store against its
constraints: per fact type it applies forml.validate_for to <pop, D> and reports the
non-empty violation sets. It was Python-only; the Rust resident now carries a native
validate (main.rs native_validate) that ports the ASSEMBLY of validate_for and reuses
the already-canon composition (system:validate_of), families (constraints:*), and
absorbed pop (system:ftpop_absorbed) via the native carrier — no Python, no cli.py.
Wired behind AREST_NATIVE_VALIDATE / apps.cli None, mirroring apps_compile and verify.

This harness compiles each app, then validates it BOTH ways and compares the receipts
fact-type by fact-type:
  * Python:  Registry.compile(name); Registry.validate(name)            -> {"app","violations"}
  * Native:  resident apps_compile -> apps_use -> validate (flagged)    -> {"app","violations"}
each violation is {fact_type, kinds, offenders, alethic}; parity means identical
fact-type sets and, per fact type, identical kinds, alethic, and offender ROW SETS
(offender order is not contractual — both build from the same validator output, but we
compare as sets so an ordering nuance is not reported as a divergence).

Companion to verify_parity.py (the audit axis) and apps_compile_parity.py (the store).
Together these de-Python the read/verify verbs. A NATIVE ERR line with code -32011 marks
the one un-ported case (absorbed exclusion-family rebuild) — honest refusal, not a wrong
receipt; such an app is reported so its coverage is visible, not silently skipped.

Usage:  python engine/tools/validate_parity.py [app1 app2 ...]
        APPS_DIR=/path/to/apps  AREST_BIN=/path/to/arest.exe   (optional)
Exit 0 iff every app's native and Python validate receipts agree.
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


def _native_validate(name, scratch):
    """Resident session: native compile -> load -> native validate. Returns
    (receipt, None) or (None, diagnostic)."""
    env = dict(os.environ)
    env["AREST_NATIVE_COMPILE"] = "1"
    env["AREST_NATIVE_VALIDATE"] = "1"

    def call(i, tool, argd):
        return json.dumps({"jsonrpc": "2.0", "id": i, "method": "tools/call",
                           "params": {"name": tool, "arguments": argd}}) + "\n"
    stdin = (call(1, "apps_compile", {"app": name})
             + call(2, "apps_use", {"name": name})
             + call(3, "validate", {}))
    p = subprocess.run([BIN, "--mcp", "--apps-dir", scratch], input=stdin,
                       capture_output=True, text=True, timeout=600, env=env)
    receipt = None
    err = None
    for line in p.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except Exception:
            continue
        if obj.get("id") == 3:
            if "error" in obj:
                err = obj["error"]
                continue
            res = obj.get("result", {})
            txt = None
            if isinstance(res, dict) and "content" in res:
                for c in res["content"]:
                    if c.get("type") == "text":
                        txt = c.get("text")
            receipt = json.loads(txt) if txt else res
    if receipt is None:
        return None, (err or (p.stdout[-400:] + " | STDERR " + p.stderr[-400:]))
    return receipt, None


def _bag(receipt):
    """fact_type -> (kinds tuple, alethic, frozenset of offender-row json)."""
    out = {}
    for v in receipt.get("violations", []):
        offenders = frozenset(json.dumps(o, sort_keys=True) for o in v.get("offenders", []))
        out[v["fact_type"]] = (tuple(v.get("kinds", [])), v.get("alethic"), offenders)
    return out


def validate_parity(names):
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
        scratch = tempfile.mkdtemp(prefix=f"vd_{name}_")
        try:
            rd = os.path.join(scratch, name, "readings")
            os.makedirs(rd)
            for f in os.listdir(src):
                if f.endswith(".md"):
                    shutil.copy(os.path.join(src, f), os.path.join(rd, f))
            A.Registry(scratch, base_dir=base).compile(name)
            py = A.Registry(scratch, base_dir=base).validate(name)
            nat, err = _native_validate(name, scratch)
            if nat is None:
                print(f"{name:24} NATIVE ERR: {err}")
                ok_all = False
                continue
            pb, nb = _bag(py), _bag(nat)
            only_py = sorted(set(pb) - set(nb))
            only_nat = sorted(set(nb) - set(pb))
            diff = [ft for ft in (set(pb) & set(nb)) if pb[ft] != nb[ft]]
            ok = not only_py and not only_nat and not diff
            ok_all = ok_all and ok
            if ok:
                print(f"{name:24} PARITY  ({len(pb)} violating fact types)")
            else:
                print(f"{name:24} DIVERGE only_py={only_py[:3]} only_nat={only_nat[:3]} "
                      f"diff={diff[:3]}")
        finally:
            shutil.rmtree(scratch, ignore_errors=True)
    print("\nVERDICT:", "VALIDATE PARITY" if ok_all else "DIVERGENCE — investigate above")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(validate_parity(sys.argv[1:] or DEFAULT_SAMPLE))
