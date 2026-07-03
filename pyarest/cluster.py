"""Partitioning across parallel instances (the platform arc): the RMAP table is the
partition unit — the writer model's stream scope — and each instance is a resident Rust
kernel that OWNS a subset of tables and retains its evolving store (the retain
protocol: a committed step's D′ replaces the owner's retained D; a refused step retains
nothing). Creates route to the owner by a stable hash of the table; reads merge by
union; and the L0/CALM property makes scatter irrelevant for monotone derivation:
union of closures = closure of union, demonstrated across real processes. This slice
covers own-table fact types; routed (absorbed) writes follow the same shape."""
import hashlib

from . import ast, forml, polyglot, system
from .lam import to_lam, from_lam
import pyarest.lam as L


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _stable(name, n):
    return int(hashlib.sha256(name.encode("utf-8")).hexdigest(), 16) % n


class Partitioned:
    def __init__(self, D, instances=2):
        self.D = D
        self.instances = instances
        self.part = system.rmap_partition(D)
        self.sessions = [polyglot.RustSession() for _ in range(instances)]
        for s in self.sessions:
            s.set_store(D)

    def owner(self, fact_type):
        return _stable(self.part.get(fact_type, fact_type), self.instances)

    def create(self, fact_type, fact, spread=None):
        """Route the create to the owning instance (or scatter deliberately with
        `spread` — the CALM test's point is that it cannot matter for L0 facts)."""
        idx = self.owner(fact_type) if spread is None else spread % self.instances
        handler = ast.build_system(cell_name=fact_type)
        ses = self.sessions[idx]
        fj = polyglot._conv(from_lam(handler))
        res = ses._rpc({"cases": [{"f": fj, "xd": polyglot._conv(list(fact)),
                                   "fuel": 0, "retain": 1}]})
        return res[0] is not None

    def _dumps(self):
        return [polyglot._untuple(s._rpc({"dump": 1})[0]) for s in self.sessions]

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
            local = system.run_rules(to_lam(dump))            # each owner's own closure
            for c in from_lam(local):
                if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL" \
                        and isinstance(c[2], tuple) \
                        and all(isinstance(r, tuple) and
                                all(not isinstance(x, tuple) for x in r) for r in c[2]):
                    merged.setdefault(c[1], set()).update(c[2])
        D = self.D
        from .reduce import apply as _ap
        for name, rows in merged.items():
            D = _ap(ast.Store(name), _S(to_lam(tuple(sorted(rows))), D))
        D = system.run_rules(D)                               # the closure of the union
        for c in from_lam(D):
            if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", head_ft):
                return set(c[2])
        return set()

    def close(self):
        for s in self.sessions:
            s.close()
