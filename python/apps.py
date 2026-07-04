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


class Registry:
    def __init__(self, apps_dir):
        self.root = os.path.abspath(apps_dir)
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

    # ---- compile: readings -> M -> lfp -> replay -> snapshot ----
    def compile(self, name):
        texts = [open(p, encoding="utf-8").read() for p in self._readings(name)]
        D, rep = forml.compile_model("\n\n".join(texts))
        D = system.run_rules(D)
        # the event log replays through the SAME create (facts are the source of
        # truth; the .db is disposable, set semantics make replay idempotent)
        if os.path.exists(self._log(name)):
            D = persist.replay(D, self._log(name))
            D = system.run_rules(D)
        persist.save_sqlite(D, self._db(name))
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

    def orient(self):
        cur = self.current()
        return {"active_app": cur, "apps": self.list()}
