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

from .canon import rust_bin

_BIN = rust_bin("arestlam")


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


# =====================================================================
# Partitioning across parallel instances (merged from cluster.py,
# 2026-07-04, the fewer-files push: the multi-instance seam IS the
# polyglot seam). The RMAP table is the partition unit, each instance a
# resident Rust kernel owning a subset of tables and retaining its
# evolving store (the retain protocol: a committed step's D-prime
# replaces the owner's retained D; a refused step retains nothing).
# Creates route by a stable hash of the table; reads merge by union;
# the L0/CALM property makes scatter irrelevant for monotone
# derivation.
# =====================================================================
import hashlib as _hashlib
from . import ast as _cl_ast, forml as _cl_forml, system as _cl_system
from .lam import to_lam, from_lam
import pyarest.lam as _cl_L


def _S(*xs):
    l = _cl_L.NIL
    for x in reversed(xs):
        l = _cl_L.CONS(x)(l)
    return _cl_L.SEQ(l)


def _stable(name, n):
    return int(_hashlib.sha256(name.encode("utf-8")).hexdigest(), 16) % n


class Partitioned:
    def __init__(self, D, instances=2):
        self.D = D
        self.instances = instances
        self.part = _cl_system.rmap_partition(D)
        self.sessions = [RustSession() for _ in range(instances)]
        for s in self.sessions:
            s.set_store(D)

    def owner(self, fact_type):
        return _stable(self.part.get(fact_type, fact_type), self.instances)

    def create(self, fact_type, fact, spread=None):
        """Route the create to the owning instance (or scatter deliberately with
        `spread` — the CALM test's point is that it cannot matter for L0 facts)."""
        idx = self.owner(fact_type) if spread is None else spread % self.instances
        handler = _cl_ast.build_system(cell_name=fact_type)
        ses = self.sessions[idx]
        fj = _conv(from_lam(handler))
        res = ses._rpc({"cases": [{"f": fj, "xd": _conv(list(fact)),
                                   "fuel": 0, "retain": 1}]})
        return res[0] is not None

    def _dumps(self):
        return [_untuple(s._rpc({"dump": 1})[0]) for s in self.sessions]

    def merged_cells(self):
        """The union read across all owners (fact cells only)."""
        out = {}
        for dump in self._dumps():
            for c in dump:
                if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL" \
                        and isinstance(c[2], tuple) \
                        and all(isinstance(r, tuple) and
                                all(not isinstance(x, tuple) for x in r) for r in c[2]):
                    out.setdefault(c[1], set()).update(c[2])
        return out

    def derive_merged(self, head_ft):
        """Union of closures = closure of union: close each instance's store locally,
        merge the fact rows, close once more globally — the same lfp regardless of the
        scatter (CALM)."""
        merged = {}
        for dump in self._dumps():
            local = _cl_system.run_rules(to_lam(dump))            # each owner's own closure
            for c in from_lam(local):
                if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL" \
                        and isinstance(c[2], tuple) \
                        and all(isinstance(r, tuple) and
                                all(not isinstance(x, tuple) for x in r) for r in c[2]):
                    merged.setdefault(c[1], set()).update(c[2])
        D = self.D
        from .reduce import apply as _ap
        for name, rows in merged.items():
            D = _ap(_cl_ast.Store(name), _S(to_lam(tuple(sorted(rows))), D))
        D = _cl_system.run_rules(D)                               # the closure of the union
        for c in from_lam(D):
            if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", head_ft):
                return set(c[2])
        return set()

    def close(self):
        for s in self.sessions:
            s.close()
