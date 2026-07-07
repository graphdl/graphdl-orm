"""Host tooling in ONE file (the seven-file shape): optimize (the
schema-level and object-level optimizers, Halpin 12.5 and Backus 12.2) and
polyglot (the Rust kernel seam and the partition protocol). Each section
keeps its own docstring; the package init aliases the old names."""

# ===================== optimize =====================
"""Conceptual schema optimization (Halpin book 12.5), decisions as derived facts.

Halpin's procedure transforms a conceptual schema so the standard Rmap yields a more
efficient implementation, and he names four judgment factors: target system, query
pattern, update pattern, clarity. Here each is data, in the pattern this system
already uses for encryption (seal derives modes from data types and constraints) and
table shapes (RMAP derives them from uniqueness constraints):

  * TRIGGERS are constraint patterns M already holds. Step 4's trigger is an
    exclusive family of unaries on one noun; the table-width-guideline/PSG1 trigger
    is a small enumerated role inside an n-ary fact type.
  * THRESHOLDS are declared facts (optThreshold rows), defaulting to Halpin's own
    "reasonable number (e.g., 5)" for enumeration width.
  * The QUERY PATTERN is the measured read log (system.read_pop), host-side like the
    event log; "focused" is a count.
  * CLARITY is moot: the authored M is never rewritten. Halpin sanctions applying
    the transforms "automatically as an invisible, preprocessing stage to Rmap", and
    that is where the apply side will live, behind a population round-trip oracle.

plan(D) is PURE analysis: suggestions ranked focused-first, each citing the M facts
that fired it (the grounds), so a firing is explainable and a disagreement is settled
by changing a declaration or reading the log. The formal lineage: equivalence and the
transformation theorems are Halpin (1989b) and Halpin & Proper (1995); the
objective-over-equivalent-schemas formulation is van Bommel & van der Weide (1992),
both cited from the book's own chapter notes."""
import re

from . import system
from .lam import to_lam, from_lam

_READ_LOG = {}


def read_pop(D, ft, partition=None):
    """THE public population read (a ρ-application over the cell or the absorbed
    view). Logged HERE, host-side, per fact type: like arrival order, the read log is
    the log's and no fact of the domain (Prop. onestep's distinction), which is also
    why it lives in this host-tooling module and not in the canonical system module —
    the optimizer's 'focused queries' are measured counts, not guesses."""
    _READ_LOG[ft] = _READ_LOG.get(ft, 0) + 1
    if partition is not None and partition.get(ft, ft) != ft:
        return system.ft_view(D, ft, partition)
    return {tuple(r) if isinstance(r, tuple) else (r,)
            for r in system._pop_rows(D, ft)}


def read_counts():
    """The measured query pattern: fact type → public reads this process."""
    return dict(_READ_LOG)


def reset_read_log():
    _READ_LOG.clear()


_QUOTED = re.compile(r"'([^']*)'")

_DEFAULTS = {"enum_width": 5}                                 # Halpin's "e.g., 5"


def _thresholds(D):
    t = dict(_DEFAULTS)
    for r in system._pop_rows(D, "optThreshold"):
        if len(r) >= 2:
            try:
                t[r[0]] = int(r[1])
            except (TypeError, ValueError):
                pass
    return t


def _enum_width(spec):
    """The width of an ENUMERATED value constraint, or None when the constraint is a
    range or bound ('at most 5', '1..9') — only a closed enumeration sanctions
    absorption (PSG1 needs the values b1..bn)."""
    if not isinstance(spec, str) or " at " in f" {spec} " or ".." in spec:
        return None
    quoted = _QUOTED.findall(spec)
    if quoted:
        return len(quoted)
    parts = [p.strip() for p in spec.split(",") if p.strip()]
    return len(parts) if parts and all(" " not in p for p in parts) else None


def plan(D, reads=None):
    """Advisory suggestions over M, ranked focused-first. Pure: nothing is rewritten,
    nothing asserted; each suggestion cites its grounds."""
    reads = reads or {}
    roles = [r for r in system._pop_rows(D, "role") if len(r) >= 4]
    arity = {}
    player = {}
    for (_rid, ft, pos, typ) in roles:
        arity[ft] = max(arity.get(ft, 0), pos)
        player[(ft, pos)] = typ
    enums = {}
    for r in system._pop_rows(D, "valueConstraint"):
        if len(r) >= 2:
            w = _enum_width(r[1])
            if w is not None:
                enums[r[0]] = w
    th = _thresholds(D)
    out = []

    # Step 4 (book 12.5): an exclusive family of unaries on one noun generalizes to a
    # single functional binary over the enumerated family; the exclusion becomes the
    # key uniqueness. Trigger: an exclusion/exclusive_or constraint whose clauses are
    # ALL unary fact types sharing their role player.
    for f in system._pop_rows(D, "constraint"):
        if len(f) >= 4 and f[1] in ("exclusion", "exclusive_or") \
                and isinstance(f[3], tuple) and len(f[3]) >= 2:
            fts = tuple(f[3])
            if all(arity.get(ft) == 1 for ft in fts):
                nouns = {player.get((ft, 1)) for ft in fts}
                if len(nouns) == 1:
                    out.append({
                        "kind": "generalize_exclusive_unaries",
                        "noun": next(iter(nouns)),
                        "fact_types": fts,
                        "reads": max(reads.get(ft, 0) for ft in fts),
                        "grounds": {"constraint": f[0], "family": len(fts)},
                    })

    # PSG1 absorption under the table width guideline (book 12.5 steps 2.1/3): a
    # small, closed enumeration playing a role in an n-ary sanctions specializing the
    # predicate by absorbing it. Width bounded by the DECLARED threshold; stability
    # (no enumeration changes in the M log's window) joins the grounds when the
    # M-history wiring lands.
    for ft, a in sorted(arity.items()):
        if a < 3:
            continue
        for pos in range(1, a + 1):
            v = player.get((ft, pos))
            w = enums.get(v)
            if w is not None and w <= th["enum_width"]:
                out.append({
                    "kind": "absorb_enumerated_role",
                    "fact_type": ft,
                    "role": pos,
                    "value_type": v,
                    "reads": reads.get(ft, 0),
                    "grounds": {"valueConstraint": v, "width": w,
                                "threshold": th["enum_width"]},
                })

    return sorted(out, key=lambda s: -s["reads"])


# =====================================================================
# The Backus-level optimizer (merged from rewrite.py, 2026-07-04, the
# fewer-files push: one module owns optimization at both levels, the
# schema level above per Halpin 12.5 and the object level below per
# Backus 12.2). HOST TOOLING by design: a rewritten object is a TWIN of
# a canonical one, held to observational equality, the same contract as
# the FAST overrides; v1 carries only the unconditionally bottom-safe
# laws (composition associativity, identity elimination, CONST
# absorption). The catalog and oracle doctrine live in
# docs/2026-07-03-backus-optimizer-catalog.md.
# =====================================================================


def _is(t, head):
    return isinstance(t, tuple) and len(t) > 0 and t[0] == head


def _flatten_comp(elems):
    out = []
    for e in elems:
        if _is(e, "COMP"):
            out.extend(_flatten_comp(list(e[1:])))
        else:
            out.append(e)
    return out


def rewrite(tree):
    """One bottom-up pass of the ⊥-safe laws over a from_lam object tree."""
    if not isinstance(tree, tuple) or not tree:
        return tree
    t = tuple(rewrite(x) for x in tree)
    if _is(t, "COMP"):
        elems = [e for e in _flatten_comp(list(t[1:])) if e != "id"]   # assoc + III.2
        if not elems:
            return "id"
        if len(elems) == 1:
            return elems[0]
        return ("COMP",) + tuple(elems)
    if _is(t, "COND") and len(t) == 4:
        p, then, els = t[1], t[2], t[3]
        if _is(then, "COND") and len(then) == 4 and then[1] == p:      # II.3.1
            return ("COND", p, then[2], els)
    return t


def twin(obj, operands, step_D=None):
    """Rewrite a compiled object and hold it to observational equality on the given
    operands (the catalog's oracle). Answers the rewritten object as a lambda value,
    or the original when the rewrite changed nothing. Raises on divergence, because
    a twin that diverges is a bug, not a fallback."""
    from . import defs
    from .reduce import apply
    import pyarest.lam as L

    tree = from_lam(obj)
    better = rewrite(tree)
    if better == tree:
        return obj
    cand = to_lam(better)
    D = step_D if step_D is not None else L.SEQ(L.NIL)
    for x in operands:
        with defs.step(D):
            got = from_lam(apply(cand, x))
            want = from_lam(apply(obj, x))
        if got != want:
            raise AssertionError(
                f"twin diverged on {from_lam(x)!r}: {got!r} != {want!r}")
    return cand


# ===================== polyglot =====================
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

_BIN = rust_bin("arest")


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
