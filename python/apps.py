"""The apps protocol (the swap contract): the same directory layout the old engine
serves — an app is <apps_dir>/<name>/ with readings/*.md and <name>.db — behind the
substrate's own machinery. An app IS a store: compiling runs every reading through
compile_model to M and run_rules to the lfp (Def. derive), and the .db is
persist.save_sqlite of the resulting store, cells one to one. Recompile is a
FROM-SCRATCH rebuild by design: the old engine's incremental compile left
superseded projected rows (its own ledger prescribes delete-and-rebuild), so here
the rebuild is the semantics and frozen ingestion makes the unchanged case cheap.
The active app is a marker file, read fresh by every registry, exactly as the old
engine's session marker behaves."""
import json
import os

from . import forml, persist, system

_MARKER = ".pyarest-active-app"


def default_base():
    """The vendored base readings directory (shared/base), or None if absent."""
    from . import paths
    d = os.path.join(paths.root(), "shared", "base")
    return d if os.path.isdir(d) else None


class Registry:
    """base_dir preloads the vendored base readings (shared/base — the old engine's
    CORE_READINGS backbone) under every app compile via frozen ingestion; the live
    MCP server passes it (apps.default_base()), so a bare Registry is a base-free
    library object and the server carries the parity default."""
    def __init__(self, apps_dir, base_dir=None, cache_dir=None):
        self.root = os.path.abspath(apps_dir)
        self.base_dir = base_dir
        self.cache_dir = cache_dir
        self.last_receipt = None                              # the context tool replays it

    # ---- inventory ----
    def _app_dir(self, name):
        return os.path.join(self.root, name)

    def _db(self, name):
        return os.path.join(self._app_dir(name), f"{name}.db")

    def _readings(self, name):
        d = os.path.join(self._app_dir(name), "readings")
        if not os.path.isdir(d):
            return []
        return [os.path.join(d, fn) for fn in sorted(os.listdir(d))
                if fn.endswith(".md")]

    def list(self):
        out = []
        for name in sorted(os.listdir(self.root)):
            d = self._app_dir(name)
            if not os.path.isdir(d) or not os.path.isdir(os.path.join(d, "readings")):
                continue
            db = self._db(name)
            out.append({
                "name": name,
                "root": d,
                "compiled": os.path.exists(db),
                "last_compile": os.path.getmtime(db) if os.path.exists(db) else None,
            })
        return out

    # ---- the active app marker ----
    def use(self, name):
        if not os.path.isdir(self._app_dir(name)):
            raise FileNotFoundError(f"no app {name!r} under {self.root}")
        with open(os.path.join(self.root, _MARKER), "w", encoding="utf-8") as f:
            f.write(name)
        return name

    def current(self):
        p = os.path.join(self.root, _MARKER)
        if not os.path.exists(p):
            return None
        return open(p, encoding="utf-8").read().strip() or None

    def _log(self, name):
        return os.path.join(self._app_dir(name), f"{name}.events.jsonl")

    def _base_D(self):
        """The preloaded base, thawed (frozen ingestion pays the compile once per
        engine fingerprint; every later registry thaws in milliseconds)."""
        if not self.base_dir or not os.path.isdir(self.base_dir):
            return None
        text = "\n\n".join(
            open(os.path.join(self.base_dir, fn), encoding="utf-8").read()
            for fn in sorted(os.listdir(self.base_dir)) if fn.endswith(".md"))
        return persist.ingest_frozen(text, cache_dir=self.cache_dir)

    # ---- compile: readings -> M -> lfp -> replay -> snapshot ----
    def compile(self, name):
        texts = [open(p, encoding="utf-8").read() for p in self._readings(name)]
        base = self._base_D()
        D, rep = forml.compile_model("\n\n".join(texts), D=base,
                                     context_from=base)
        D = system.run_rules(D)
        # the event log replays through the SAME create (facts are the source of
        # truth; the .db is disposable, set semantics make replay idempotent)
        if os.path.exists(self._log(name)):
            D = persist.replay(D, self._log(name))
            D = system.run_rules(D)
        persist.save_sqlite(D, self._db(name))
        # the RMAP projection rides in the same .db (the GraphDL contract): the
        # relational tables downstream SQL consumers read, beside the cells
        import sqlite3
        from . import ddl
        con = sqlite3.connect(self._db(name))
        try:
            rep["projected"] = ddl.project(D, con)
        finally:
            con.close()
        rep["app"] = name
        return rep

    # ---- the write side: eq. create against the app's store ----
    def apply(self, name, fact_type, fact):
        """One create against the app's store: validate over the derived candidate,
        commit iff no alethic violation (eq. create), append the committed step to
        the event log (a refusal appends nothing), snapshot, and answer the RECEIPT:
        committed, the violation set, and the representation parts."""
        from .lam import to_lam, from_lam, atom as _A
        from .reduce import apply as _ap
        D = self._load(name)
        row = tuple(fact)
        res = system.create(D, fact_type, to_lam(row))
        o = from_lam(_ap(_A(1), res))
        D2 = _ap(_A(2), res)
        refused = o == "ERROR" or from_lam(D2) == from_lam(D)
        violations = []
        if isinstance(o, tuple) and len(o) >= 2 and isinstance(o[1], tuple):
            violations = [list(v) for v in o[1]]
        elif refused:
            # create answers the bare ERROR atom on an alethic refusal; the receipt
            # still owes the offenders (Def. Violation: the message is V), so run
            # the validate over the candidate population directly
            from . import defs
            import pyarest.lam as L
            val = forml.validate_for(fact_type, D, system.rmap_partition(D))
            cand = tuple(tuple(r) for r in system._pop_rows(D, fact_type)) + (row,)
            pair = L.SEQ(L.CONS(to_lam(cand))(L.CONS(D)(L.NIL)))
            with defs.step(D):
                _p, v, _f = from_lam(_ap(val, pair))
            violations = [list(x) if isinstance(x, tuple) else [x] for x in v]
        if not refused:
            D2 = system.run_rules(D2, changed=[fact_type])
            with open(self._log(name), "a", encoding="utf-8") as f:
                f.write(json.dumps({"ft": fact_type, "fact": list(row)},
                                   ensure_ascii=False) + "\n")
            persist.save_sqlite(D2, self._db(name))
        receipt = {"app": name, "fact_type": fact_type, "fact": list(row),
                   "committed": not refused, "violations": violations}
        self.last_receipt = receipt
        return receipt

    def retract(self, name, fact_type, fact):
        """Logical deletion, validated: the SHRUNK candidate population must satisfy
        the schema (a retract can violate mandatory and frequency lower bounds, so
        it refuses exactly like a create — Def. Violation is direction-blind). On
        commit the retraction is a LOG entry and the store rebuilds through compile,
        so derived rows recompute from scratch (the supersession discipline)."""
        from .lam import to_lam, from_lam
        from .reduce import apply as _ap
        from . import defs
        import pyarest.lam as L
        D = self._load(name)
        row = tuple(fact)
        pop = {tuple(r) for r in system._pop_rows(D, fact_type)}
        if row not in pop:
            receipt = {"app": name, "fact_type": fact_type, "fact": list(row),
                       "committed": False, "violations": [],
                       "note": "no such fact"}
            self.last_receipt = receipt
            return receipt
        cand = tuple(sorted(pop - {row}))
        val = forml.validate_for(fact_type, D, system.rmap_partition(D))
        pair = L.SEQ(L.CONS(to_lam(cand))(L.CONS(D)(L.NIL)))
        with defs.step(D):
            _p, v, flag = from_lam(_ap(val, pair))
        if flag == "T":
            receipt = {"app": name, "fact_type": fact_type, "fact": list(row),
                       "committed": False,
                       "violations": [list(x) if isinstance(x, tuple) else [x]
                                      for x in v]}
            self.last_receipt = receipt
            return receipt
        with open(self._log(name), "a", encoding="utf-8") as f:
            f.write(json.dumps({"op": "retract", "ft": fact_type,
                                "fact": list(row)}, ensure_ascii=False) + "\n")
        self.compile(name)                                    # rebuild: log applied
        receipt = {"app": name, "fact_type": fact_type, "fact": list(row),
                   "committed": True, "violations": []}
        self.last_receipt = receipt
        return receipt

    # ---- reads ----
    def _load(self, name):
        db = self._db(name)
        if not os.path.exists(db):
            raise FileNotFoundError(f"app {name!r} is not compiled (no {db})")
        return persist.load_sqlite(db)

    def query(self, name, fact_type):
        D = self._load(name)
        return [tuple(r) for r in system._pop_rows(D, fact_type)]

    def sql(self, name, statement):
        import sqlite3
        con = sqlite3.connect(self._db(name))
        try:
            return con.execute(statement).fetchall()
        finally:
            con.close()

    def get(self, name, noun, entity_id):
        """The 3NF per-entity view: the key, absorbed functional values, unary
        booleans, and every own-table fact the id participates in — the same
        grouping the projection uses (one rmap, two consumers)."""
        from . import ddl
        D = self._load(name)
        partition, roles, ref, entities, _mand = ddl._analyze(D)
        own = [ft for ft, key in partition.items() if key == ft]
        entity_tables = entities | ({t for t in partition.values()} - set(own))
        fields, facts, seen = {}, [], False
        for (ft, col, kind, other) in ddl._entity_columns(
                noun, partition, roles, ref, entities, entity_tables):
            rows = [tuple(r) for r in system._pop_rows(D, ft)]
            if kind == "unary":
                fields[col] = any(r and r[0] == entity_id for r in rows)
                seen = seen or fields[col]
            else:
                val = {r[0]: r[1] for r in rows if len(r) >= 2}
                fields[other or col] = val.get(entity_id)     # the played type names the field
                seen = seen or entity_id in val
        for ft in own:
            rs = roles.get(ft, [])
            for row in system._pop_rows(D, ft):
                row = tuple(row)
                if any(p <= len(row) and row[p - 1] == entity_id
                       for (p, player) in rs if player == noun):
                    facts.append({"fact_type": ft, "row": list(row)})
                    seen = True
        seen = seen or any(r and r[0] == entity_id
                           for r in system._pop_rows(D, noun))
        return {"app": name, "noun": noun, "id": entity_id,
                "exists": bool(seen), "fields": fields,
                "facts": facts}

    def cells(self, name, pattern=None, cell=None):
        """The store surface: cell names with row counts, or one cell's rows."""
        D = self._load(name)
        if cell:
            return [list(r) for r in system._pop_rows(D, cell)]
        out = []
        for f in system._pop_rows(D, "factType"):
            if not f:
                continue
            nm = f[0]
            if pattern and pattern.lower() not in nm.lower():
                continue
            out.append({"name": nm, "rows": len(system._pop_rows(D, nm))})
        return sorted(out, key=lambda c: c["name"])

    def schema(self, name):
        """The model surface: object types, fact types with readings and roles,
        and the constraint inventory."""
        D = self._load(name)
        nouns = [{"name": r[0], "kind": r[1]}
                 for r in system._pop_rows(D, "instanceOf")
                 if len(r) >= 2 and r[1] in ("ObjectType", "ValueType")]
        roles = {}
        for r in system._pop_rows(D, "role"):
            if len(r) >= 4:
                roles.setdefault(r[1], []).append((r[2], r[3]))
        fts = []
        for f in system._pop_rows(D, "factType"):
            if len(f) >= 2:
                fts.append({"id": f[0], "reading": f[1],
                            "roles": [p for (_i, p) in sorted(roles.get(f[0], []))]})
        cons = [{"id": c[0], "kind": c[1],
                 "fact_type": c[2] if len(c) > 2 else None}
                for c in system._pop_rows(D, "constraint") if len(c) >= 2]
        return {"app": name, "object_types": sorted(nouns, key=lambda n: n["name"]),
                "fact_types": sorted(fts, key=lambda f: f["id"]),
                "constraints": cons}

    def orient(self):
        cur = self.current()
        return {"active_app": cur, "apps": self.list()}
