"""The polyglot seam (rust/src/main.rs is the other half): everything above the lambda
kernel is a VALUE, so it travels. A scenario carries a store D, the compiled process
definitions, and ⟨f, x, fuel⟩ cases as JSON; the Rust kernel — the same Scott union and
the same Y-built mu on Rc closures — reduces them; the differential asserts agreement
with the Python Scott mu (ground truth). The port surface is exactly prims.BASE plus
DEFS and cellkey: Cor. boundary makes the polyglot enumerable, and the machines, the
constraints, and M itself ride across as data with no Rust written for them."""
import json
import os
import subprocess

from . import defs
from .lam import from_lam

_BIN = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                    "rust", "target", "release",
                    "arestlam.exe" if os.name == "nt" else "arestlam")


def _conv(v):
    if isinstance(v, tuple):
        return [_conv(x) for x in v]
    return v


def _untuple(v):
    if isinstance(v, list):
        return tuple(_untuple(x) for x in v)
    return v


def export_scenario(D, cases):
    """D, the compiled process defs, and ⟨f, x, fuel⟩ cases as the wire scenario."""
    process = [[n, _conv(from_lam(obj))]
               for n, (kind, obj) in defs.latest.items() if kind == "compiled"]
    return {"d": _conv(from_lam(D)),
            "overrides": 1,
            "process": process,
            "cases": [{"f": _conv(from_lam(f)), "x": _conv(from_lam(x)), "fuel": fuel or 0}
                      for (f, x, fuel) in cases]}


def rust_available():
    return os.path.exists(_BIN)


def run_rust(scenario, timeout=600):
    """Reduce the scenario's cases under the Rust kernel; '⊥' marks bottom (the same
    marker Python's from_lam uses), so results compare directly against ground truth."""
    res = subprocess.run([_BIN],
                         input=json.dumps(scenario, ensure_ascii=False).encode("utf-8"),
                         capture_output=True, timeout=timeout)
    if res.returncode != 0:
        raise RuntimeError(res.stderr.decode("utf-8", "replace"))
    out = []
    for line in res.stdout.decode("utf-8").splitlines():
        v = json.loads(line)
        out.append("⊥" if v is None else _untuple(v))
    return out


def python_ground_truth(D, cases):
    """The same cases under the Python Scott mu (reduce.apply_lambda), per-case frames."""
    from .reduce import apply_lambda
    from .lam import to_lam
    import pyarest.lam as L
    out = []
    for (f, x, fuel) in cases:
        with defs.step(D, fuel):
            out.append(from_lam(apply_lambda(f, x)))
    return out


class RustSession:
    """The resident runner: one spawned kernel serving scenario lines over stdio, the
    store retained across requests (set_store once, then cases reference it via the
    xd protocol — ⟨fact, D⟩ without re-serializing D). Amortizes spawn AND store
    serialization, so timings isolate reduction."""

    def __init__(self):
        self.proc = subprocess.Popen([_BIN, "--serve"], stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE)

    def _rpc(self, obj):
        line = json.dumps(obj, ensure_ascii=False) + "\n"
        self.proc.stdin.write(line.encode("utf-8"))
        self.proc.stdin.flush()
        out = self.proc.stdout.readline().decode("utf-8")
        return json.loads(out)

    def set_store(self, D, overrides=True):
        process = [[n, _conv(from_lam(obj))]
                   for n, (kind, obj) in defs.latest.items() if kind == "compiled"]
        self._rpc({"d": _conv(from_lam(D)), "process": process,
                   "overrides": 1 if overrides else 0, "cases": []})

    def run_facts(self, f, facts, fuel=None, engine=None):
        """Apply `f` to ⟨fact, D_retained⟩ per fact — the machine-step shape. `engine`
        selects the evaluator ("native" = the deepest override; default the Scott
        closures), certified equal by the differential."""
        fj = _conv(from_lam(f))
        req = {"cases": [{"f": fj, "xd": _conv(from_lam(x)), "fuel": fuel or 0}
                         for x in facts]}
        if engine:
            req["engine"] = engine
        res = self._rpc(req)
        return ["⊥" if v is None else _untuple(v) for v in res]

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=10)
        except Exception:
            self.proc.kill()
