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

    # ---- compile: readings -> M -> lfp -> snapshot ----
    def compile(self, name):
        texts = [open(p, encoding="utf-8").read() for p in self._readings(name)]
        D, rep = forml.compile_model("\n\n".join(texts))
        D = system.run_rules(D)
        persist.save_sqlite(D, self._db(name))
        rep["app"] = name
        return rep

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
