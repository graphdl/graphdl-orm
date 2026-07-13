#!/usr/bin/env python3
"""Runtime certified-equality check for the certified-TWIN overrides — the second
equality axis of the resolution registry (docs/15-resolution-registry.md).

The registry pattern: canon defines meaning; the host carries a FAST TWIN of a hot
def (the theta join/dedup arms, vb_fetch, entity_view, ...) gated by a kill switch;
the twin CLAIMS byte-equality to its canon DEF of record. This harness turns that
claim into a RUNTIME FACT on real app compiles:

  native apps_compile  with the fast twins ON   (default)
  vs
  native apps_compile  with the twins FLIPPED to their canon DEFs (AREST_NO_THETA_ARMS=1)

flipping the switch makes `fn prim` return None for the theta arms, so NEval defers
to the shared/*.canon DEF (the interpreter path) — same store, more slowly. We
byte-compare the two NATIVE stores cell by cell and also TIME both: twins-off must
be measurably slower, which proves the arms actually fired (a non-vacuous test — if
an app's compile never touches a join/dedup, EQUAL would be trivially true).

This is the companion to `apps_compile_parity.py`:
  * apps_compile_parity.py  proves  native  == python      (cross-HOST equality)
  * twin_equality.py        proves  fast-twin == canon-DEF  (intra-host OVERRIDE equality)

Both are "no Python required to be CORRECT" guarantees from opposite directions:
the parity harness needs Python present (it IS the reference); this one needs only
the Rust binary and the shared canon — it certifies the fast path against the slow
canon reference with no Python in the loop at all.

Usage:
  python engine/tools/twin_equality.py [app1 app2 ...]     # default: a diverse sample
  APPS_DIR=/path/to/apps  AREST_BIN=/path/to/arest.exe     # optional overrides

Exit 0 iff every app's twins-on store is byte-identical to its twins-off store.
Run it after ANY change to a certified-twin arm in `fn prim` (cooks.rs / main.rs)
or to a theta canon DEF (arest.canon) — a divergence here means a twin has drifted
from its canon meaning, which no cross-host parity run would catch (both hosts could
share the same fast twin and the same drift).
"""
import os
import sys
import json
import time
import shutil
import tempfile
import subprocess

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # engine/
APPS_DIR = os.environ.get("APPS_DIR", os.path.join(os.path.dirname(_ROOT), "..", "apps"))
BIN = os.environ.get("AREST_BIN", os.path.join(_ROOT, "rust", "target", "release", "arest.exe"))

DEFAULT_SAMPLE = ["listings-vdp", "spd-guardian", "message-vetting", "charge-dispute-service"]


def _cellmap(store):
    out = {}
    for c in store.get("d", []):
        if isinstance(c, list) and len(c) >= 3 and c[0] == "CELL":
            rows = c[2]
            out[c[1]] = ([json.dumps(r, sort_keys=True) for r in rows]
                         if isinstance(rows, list) else rows)
    return out


def _native_compile(name, scratch, twins_off):
    """Compile `name` natively (no Python) into scratch; return (store, seconds).
    twins_off flips every certified theta arm to its canon DEF of record."""
    env = dict(os.environ)
    env["AREST_NATIVE_COMPILE"] = "1"
    if twins_off:
        env["AREST_NO_THETA_ARMS"] = "1"
    else:
        env.pop("AREST_NO_THETA_ARMS", None)
    nf = os.path.join(scratch, name, name + ".store.json")
    if os.path.exists(nf):
        os.remove(nf)
    call = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": "apps_compile", "arguments": {"app": name}}}) + "\n"
    t0 = time.time()
    subprocess.run([BIN, "--mcp", "--apps-dir", scratch], input=call,
                   capture_output=True, text=True, timeout=600, env=env)
    dt = time.time() - t0
    if not os.path.exists(nf):
        return None, dt
    return json.load(open(nf, encoding="utf-8")), dt


def twin_equality(apps_names):
    ok_all = True
    for name in apps_names:
        src_rd = os.path.join(APPS_DIR, name, "readings")
        if not os.path.isdir(src_rd):
            print(f"{name:28} SKIP (no readings/)")
            continue
        scratch = tempfile.mkdtemp(prefix=f"twineq_{name}_")
        try:
            rd = os.path.join(scratch, name, "readings")
            os.makedirs(rd)
            for f in os.listdir(src_rd):
                if f.endswith(".md"):
                    shutil.copy(os.path.join(src_rd, f), os.path.join(rd, f))
            on, t_on = _native_compile(name, scratch, twins_off=False)
            off, t_off = _native_compile(name, scratch, twins_off=True)
            if on is None or off is None:
                print(f"{name:28} NO OUTPUT (on={on is not None} off={off is not None})")
                ok_all = False
                continue
            om, fm = _cellmap(on), _cellmap(off)
            ok_keys, fk = set(om), set(fm)
            content = [k for k in (ok_keys & fk) if om[k] != fm[k]]
            missing = sorted(ok_keys - fk)
            extra = sorted(fk - ok_keys)
            ok = not missing and not extra and not content
            ok_all = ok_all and ok
            # non-vacuity signal: canon path must be slower if the arms fired
            fired = ("arms fired" if t_off > t_on * 1.05
                     else "theta unused here — EQUAL is vacuous for this app")
            if ok:
                print(f"{name:28} EQUAL  (cells={len(ok_keys)}) "
                      f"t_on={t_on:.2f}s t_off={t_off:.2f}s :: {fired}")
            else:
                print(f"{name:28} DIVERGE only_on={missing[:3]} "
                      f"only_off={extra[:3]} content={content[:3]}")
        finally:
            shutil.rmtree(scratch, ignore_errors=True)
    print("\nVERDICT:", "TWINS CERTIFIED-EQUAL TO CANON AT RUNTIME" if ok_all
          else "DIVERGENCE — a twin is NOT byte-equal to its canon DEF (investigate above)")
    return 0 if ok_all else 1


if __name__ == "__main__":
    names = sys.argv[1:] or DEFAULT_SAMPLE
    sys.exit(twin_equality(names))
