"""Persistence drivers (the platform arc): the paper names the binding ("a server
registers httpFetch and upsert"), so a driver is host machinery behind a small
interface, and the EVENT LOG is the primary durable form — each committed step appended
as ⟨tx, fact_type, fact⟩, the τ log made durable, with cell snapshots as replay
optimizations. Facts are the source of truth: replay is re-ingestion through the SAME
create, and populations being sets makes replay idempotent. sqlite maps the cell model
one-to-one (a cells table, one row per cell, per-step transactions matching the atomic
commit); jsonl carries the log for audit and replication."""
import json
import os
import sqlite3

from . import ast, system
from .lam import to_lam, from_lam, atom as _A
from .reduce import apply as _ap
import pyarest.lam as L


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _conv(v):
    if isinstance(v, tuple):
        return [_conv(x) for x in v]
    return v


def _untuple(v):
    if isinstance(v, list):
        return tuple(_untuple(x) for x in v)
    return v


# ============================ sqlite: cells one-to-one ========================
def save_sqlite(D, path, seal_key=None):
    """Snapshot every cell (fact populations AND definitions — both are data) into a
    cells table, insertion order preserved (first match wins on load, as in D). With a
    seal_key, the roles the schema derives as sensitive (seal.plan) are sealed before
    they touch disk — field-level encryption at rest, mode per constraint."""
    from . import seal as _seal
    sealing = _seal.plan(D)["roles"] if seal_key else {}
    bycol = {}
    for ((ft, pos), mode) in sealing.items():
        bycol.setdefault(ft, []).append((pos, mode))
    con = sqlite3.connect(path)
    try:
        con.execute("CREATE TABLE IF NOT EXISTS cells (ord INTEGER PRIMARY KEY, "
                    "name TEXT, contents TEXT)")
        con.execute("DELETE FROM cells")
        for i, c in enumerate(from_lam(D)):
            if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL":
                contents = c[2]
                if seal_key and c[1] in bycol and isinstance(contents, tuple):
                    contents = _seal.seal_rows(seal_key, contents, bycol[c[1]])
                con.execute("INSERT INTO cells (ord, name, contents) VALUES (?, ?, ?)",
                            (i, json.dumps(c[1]), json.dumps(_conv(contents),
                                                             ensure_ascii=False)))
        con.commit()
    finally:
        con.close()


def load_sqlite(path, seal_key=None):
    from . import seal as _seal
    con = sqlite3.connect(path)
    try:
        rows = con.execute("SELECT name, contents FROM cells ORDER BY ord").fetchall()
    finally:
        con.close()
    cells = tuple(("CELL", json.loads(n), _untuple(json.loads(c))) for (n, c) in rows)
    if seal_key:
        cells = tuple(
            (t, n, _seal.unseal_rows(seal_key, v)
             if isinstance(v, tuple) and all(isinstance(r, tuple) for r in v) else v)
            for (t, n, v) in cells)
    return to_lam(cells)


# ==================== frozen ingestion: thaw instead of re-ingest =============
def _cache_dir():
    base = os.environ.get("PYAREST_CACHE") or os.environ.get("LOCALAPPDATA")
    if base:
        return os.path.join(base, "pyarest") if "pyarest" not in base else base
    return os.path.join(os.path.expanduser("~"), ".cache", "pyarest")


_ENGINE_FP = []


def _engine_fingerprint():
    """A hash over the engine's own sources, BOTH strata (this host package and the
    canonical modules in shared/): a thawed D carries COMPILED objects, so an engine
    change must invalidate every snapshot — text alone would serve stale compiled
    rules silently after a compiler edit."""
    if not _ENGINE_FP:
        import hashlib
        from . import paths
        h = hashlib.sha256()
        pkg = os.path.dirname(os.path.abspath(__file__))
        for d in (pkg, os.path.join(paths.root(), "shared")):
            for fn in sorted(os.listdir(d)):
                if fn.endswith(".py"):
                    h.update(fn.encode())
                    h.update(open(os.path.join(d, fn), "rb").read())
        _ENGINE_FP.append(h.hexdigest()[:16])
    return _ENGINE_FP[0]


def ingest_frozen(text, cache_dir=None):
    """Compile `text` THROUGH the local persistence model: the compiled D freezes to a
    content-keyed sqlite snapshot, and the same text thereafter THAWS from disk instead
    of re-ingesting (definitions are data, so the snapshot carries the rules). The key
    hashes the text AND the engine's own sources: changed text or a changed compiler is
    a different snapshot, so invalidation is by construction. Writes are
    tmp-then-rename, so racing processes cannot tear one."""
    import hashlib
    from . import forml
    d = cache_dir or _cache_dir()
    key = hashlib.sha256((_engine_fingerprint() + "\x00" +
                          text).encode("utf-8")).hexdigest()[:24]
    snap = os.path.join(d, f"ingest-{key}.sqlite")
    if os.path.exists(snap):
        return load_sqlite(snap)
    D = forml.compile_model(text)[0]
    os.makedirs(d, exist_ok=True)
    tmp = snap + f".tmp{os.getpid()}"
    save_sqlite(D, tmp)
    os.replace(tmp, snap)
    return D


# ============================ jsonl: the durable step log =====================
def read_log(path):
    if not os.path.exists(path):
        return []
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


class JsonlLog:
    """The durable event log: each COMMITTED step appended as ⟨tx, ft, fact⟩ (a refused
    step appends nothing — ERROR commits nothing, so it persists nothing). tx is the
    arrival order at this log, the writer model's transaction time."""

    def __init__(self, path):
        self.path = path
        self.tx = len(read_log(path))

    def create(self, D, fact_type, fact, validate_obj=None):
        if validate_obj is None:
            res = system.create(D, fact_type, to_lam(fact))
        else:
            res = ast.run(to_lam(fact), D, validate_obj=validate_obj, cell_name=fact_type)
        o = from_lam(_ap(_A(1), res))
        D2 = _ap(_A(2), res)
        # a step commits or it doesn't: ERROR answers unchanged D, and an alethic
        # refusal commits nothing either — only a CHANGED state is a logged event
        if o == "ERROR" or from_lam(D2) == from_lam(D):
            return D2
        self.tx += 1
        with open(self.path, "a", encoding="utf-8") as f:
            f.write(json.dumps({"tx": self.tx, "ft": fact_type, "fact": _conv(fact)},
                               ensure_ascii=False) + "\n")
        return D2


def replay(D, path):
    """Rebuild state by re-ingesting the log through the SAME create (facts are the
    source of truth; set semantics make replay idempotent)."""
    for entry in read_log(path):
        D = _ap(_A(2), system.create(D, entry["ft"], to_lam(_untuple(entry["fact"]))))
    return D
