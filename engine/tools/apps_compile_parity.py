#!/usr/bin/env python3
"""Standing byte-parity acceptance check: native `apps_compile` vs Python `Registry.compile`,
over real apps, THROUGH THE REAL FLOW.

This is the reliable re-verification harness for #20 (native compile). Run it after ANY change
to the compile path (Rust cooks/op_compile_model/create_handlers/native_apps_compile, or the
Python compiler) to confirm the native store is byte-identical to the Python reference.

CRITICAL — why this harness and not a hand-rolled one:
  The native and Python compiles must be fed IDENTICAL input the way the PRODUCTION path does:
  native `apps_compile` reads `<app>/readings/*.md` itself (sorted, newlines preserved) and
  Python `Registry.compile` reads the same files the same way. Do NOT pre-compose the text with
  `open(f).read().replace("\\n", " ")` — that inlines markdown `#` headers/comment lines into
  adjacent statements and corrupts them, manufacturing a phantom divergence (this bit us
  2026-07-13; see the ledger). Both sides here go through the real registry/resident flow.

Usage:
  python engine/tools/apps_compile_parity.py [app1 app2 ...]     # default: a diverse sample
  APPS_DIR=/path/to/apps  ARERT_BIN=/path/to/arest.exe  (optional overrides)

Exit code 0 iff every app is byte-identical (semantic note: divergences are reported cell by
cell; a row-ORDER-only divergence is flagged distinctly since order is a FastStore-twin concern,
not a compile-correctness one — but this harness reports it so you decide).
"""
import importlib.util, os, sys, json, subprocess, shutil, tempfile

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # engine/
APPS_DIR = os.environ.get("APPS_DIR", os.path.join(os.path.dirname(_ROOT), "..", "apps"))
BIN = os.environ.get("AREST_BIN", os.path.join(_ROOT, "rust", "target", "release", "arest.exe"))

# a diverse default sample across sizes/constructs; override by passing app names as argv
DEFAULT_SAMPLE = ["listings-vdp", "spd-guardian", "message-vetting", "charge-dispute-service"]


def _load_pyarest():
    spec = importlib.util.spec_from_file_location(
        "pyarest", os.path.join(_ROOT, "python", "__init__.py"),
        submodule_search_locations=[os.path.join(_ROOT, "python")])
    m = importlib.util.module_from_spec(spec)
    sys.modules["pyarest"] = m
    spec.loader.exec_module(m)
    return m


def _cellmap(store):
    """cell name -> its rows as a sorted JSON list (set view) and an ordered JSON list."""
    out = {}
    for c in store.get("d", []):
        if isinstance(c, list) and len(c) >= 3 and c[0] == "CELL":
            rows = c[2]
            out[c[1]] = ([json.dumps(r, sort_keys=True) for r in rows]
                         if isinstance(rows, list) else rows)
    return out


def parity(apps_names):
    _load_pyarest()
    import pyarest.prims  # noqa: F401
    from pyarest import apps as A
    base = A.default_base()
    ok_all = True
    for name in apps_names:
        src_rd = os.path.join(APPS_DIR, name, "readings")
        if not os.path.isdir(src_rd):
            print(f"{name:28} SKIP (no readings/)")
            continue
        scratch = tempfile.mkdtemp(prefix=f"parity_{name}_")
        try:
            rd = os.path.join(scratch, name, "readings")
            os.makedirs(rd)
            for f in os.listdir(src_rd):
                if f.endswith(".md"):
                    shutil.copy(os.path.join(src_rd, f), os.path.join(rd, f))
            # the event sink is part of the compile's input (Registry.compile
            # replays it; native apps_compile passes it as replay_path) — a
            # scratch without it certifies only the replay-less path, which is
            # how the native replay omission escaped the corpus cert.
            ev = os.path.join(APPS_DIR, name, name + ".events.jsonl")
            if os.path.isfile(ev) and os.path.getsize(ev) > 0:
                shutil.copy(ev, os.path.join(scratch, name, name + ".events.jsonl"))
            # Python reference (real flow: Registry reads readings/ itself)
            A.Registry(scratch, base_dir=base).compile(name)
            py = json.load(open(os.path.join(scratch, name, name + ".store.json"), encoding="utf-8"))
            shutil.move(os.path.join(scratch, name, name + ".store.json"),
                        os.path.join(scratch, name, name + ".py.json"))
            # Native (real flow: apps_compile reads readings/ itself; no Python)
            env = dict(os.environ); env["AREST_NATIVE_COMPILE"] = "1"
            call = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                               "params": {"name": "apps_compile", "arguments": {"app": name}}}) + "\n"
            subprocess.run([BIN, "--mcp", "--apps-dir", scratch], input=call,
                           capture_output=True, text=True, timeout=300, env=env)
            nf = os.path.join(scratch, name, name + ".store.json")
            if not os.path.exists(nf):
                print(f"{name:28} NATIVE NO OUTPUT"); ok_all = False; continue
            nat = json.load(open(nf, encoding="utf-8"))
            pm, nm = _cellmap(py), _cellmap(nat)
            pk, nk = set(pm), set(nm)
            content = [k for k in (pk & nk) if pm[k] != nm[k]]
            order_only = [k for k in content
                          if sorted(pm[k]) == sorted(nm[k]) and pm[k] != nm[k]]
            real = [k for k in content if k not in order_only]
            missing = sorted(pk - nk); extra = sorted(nk - pk)
            ok = not missing and not extra and not real
            ok_all = ok_all and ok
            if ok and not order_only:
                print(f"{name:28} PARITY  (py={len(pk)} nat={len(nk)})")
            elif ok and order_only:
                print(f"{name:28} PARITY* (row-order only in {order_only[:4]})")
            else:
                print(f"{name:28} DIVERGE only_py={missing[:4]} only_nat={extra[:4]} content={real[:4]}")
        finally:
            shutil.rmtree(scratch, ignore_errors=True)
    print("\nVERDICT:", "ALL PARITY" if ok_all else "DIVERGENCE(S) — investigate above")
    return 0 if ok_all else 1


if __name__ == "__main__":
    names = sys.argv[1:] or DEFAULT_SAMPLE
    sys.exit(parity(names))
