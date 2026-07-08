"""The protocol surface in ONE file (the seven-file shape): persist (the
event log, freeze/thaw, sealing at rest), ddl (the RMAP projection),
migrate (the swap tool), federate (external vocabularies through the same
front door), apps (the registry and the read/write protocol), and
mcp_server (the stdio binding). Each section keeps its docstring; the
package init aliases the old names, and lazy in-body imports resolve
through those aliases at call time."""
import sys as _psys

# the six sections are one namespace now: sibling references resolve to
# this module itself
persist = ddl = migrate = federate = apps = mcp_server = _psys.modules[__name__]

# ===================== persist =====================
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
import zlib

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


# ==================== sqlite: cells one-to-one, symbols once ==================
def _sym_encode(doc, index, texts):
    """Interned encoding: lists stay lists, every LEAF becomes the id of its
    symbol. Symbol text is the leaf's own JSON, so int/str/bool/float round
    trip exactly."""
    if isinstance(doc, list):
        return [_sym_encode(x, index, texts) for x in doc]
    key = json.dumps(doc, ensure_ascii=False)
    i = index.get(key)
    if i is None:
        i = len(texts)
        index[key] = i
        texts.append(key)
    return i


def _sym_decode(doc, texts):
    if isinstance(doc, list):
        return tuple(_sym_decode(x, texts) for x in doc)
    return json.loads(texts[doc])


def save_sqlite(D, path, seal_key=None, compress=None):
    """Snapshot every cell (fact populations AND definitions — both are data)
    into a cells table, insertion order preserved (first match wins on load,
    as in D). SYMBOLS BY DEFAULT (Samuel 2026-07-08): every distinct leaf
    atom is stored once in a symbols table and cells reference it by id —
    the cross-cell repetition (hole markers, statuses, the compiled defs'
    combinator spellings) collapses ~2x, and the db stays join-inspectable.
    COMPRESSION IS OPT-IN (AREST_DB_COMPRESS=zlib, or the compress kwarg):
    another ~2.8x on the fleet's boards, at the cost of an opaque contents
    column. With a seal_key, the roles the schema derives as sensitive
    (seal.plan) are sealed before they touch disk — field-level encryption
    at rest, mode per constraint."""
    if compress is None:
        compress = os.environ.get("AREST_DB_COMPRESS", "") or "none"
    sealing = seal_plan(D)["roles"] if seal_key else {}
    bycol = {}
    for ((ft, pos), mode) in sealing.items():
        bycol.setdefault(ft, []).append((pos, mode))
    con = sqlite3.connect(path)
    try:
        # replace, never assume: a pre-existing db (another format era's)
        # may carry tables of another shape; ours is the contract
        con.execute("DROP TABLE IF EXISTS cells")
        con.execute("DROP TABLE IF EXISTS symbols")
        con.execute("DROP TABLE IF EXISTS format")
        con.execute("CREATE TABLE cells (ord INTEGER PRIMARY KEY, "
                    "name TEXT, contents TEXT)")
        con.execute("CREATE TABLE symbols (id INTEGER PRIMARY KEY, text TEXT)")
        con.execute("CREATE TABLE format (key TEXT PRIMARY KEY, value TEXT)")
        index, texts = {}, []
        for i, c in enumerate(from_lam(D)):
            if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL":
                contents = c[2]
                if seal_key and c[1] in bycol and isinstance(contents, tuple):
                    contents = seal_rows(seal_key, contents, bycol[c[1]])
                enc = json.dumps(_sym_encode(_conv(contents), index, texts),
                                 separators=(",", ":"), ensure_ascii=False)
                payload = (zlib.compress(enc.encode("utf-8"), 6)
                           if compress == "zlib" else enc)
                con.execute("INSERT INTO cells (ord, name, contents) "
                            "VALUES (?, ?, ?)",
                            (i, json.dumps(c[1]), payload))
        con.executemany("INSERT INTO symbols (id, text) VALUES (?, ?)",
                        list(enumerate(texts)))
        con.execute("INSERT INTO format (key, value) VALUES ('encoding', "
                    "'symbolic-v1')")
        con.execute("INSERT INTO format (key, value) VALUES ('compress', ?)",
                    (compress,))
        con.commit()
    finally:
        con.close()


def load_sqlite(path, seal_key=None):
    """The symbolic format only — no legacy reads (no paying customers, no
    legacy): a pre-symbols db raises sqlite's own no-such-table, and the
    remedy is recompile."""
    con = sqlite3.connect(path)
    try:
        rows = con.execute("SELECT name, contents FROM cells ORDER BY ord").fetchall()
        texts = [t for (t,) in
                 con.execute("SELECT text FROM symbols ORDER BY id")]
        fmt = dict(con.execute("SELECT key, value FROM format"))
    finally:
        con.close()

    def body(c):
        if fmt.get("compress") == "zlib":
            c = zlib.decompress(c).decode("utf-8")
        return _sym_decode(json.loads(c), texts)

    cells = tuple(("CELL", json.loads(n), body(c)) for (n, c) in rows)
    if seal_key:
        cells = tuple(
            (t, n, unseal_rows(seal_key, v)
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
        from . import canon as paths
        h = hashlib.sha256()
        pkg = os.path.dirname(os.path.abspath(__file__))
        for d in (pkg, os.path.join(paths.root(), "shared")):
            for fn in sorted(os.listdir(d)):
                if fn.endswith(".py"):
                    h.update(fn.encode())
                    h.update(open(os.path.join(d, fn), "rb").read())
        _ENGINE_FP.append(h.hexdigest()[:16])
    return _ENGINE_FP[0]


def ingest_frozen(text, cache_dir=None, compiler=None):
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
    D = (compiler or forml.compile_model)(text)[0]
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
    source of truth; set semantics make replay idempotent). A retract entry removes
    its row from the base population; derived rows recompute in the caller's
    run_rules pass after replay."""
    return replay_entries(D, read_log(path))


def replay_entries(D, entries):
    """Replay an event ENTRY LIST, sink-agnostic: the file sink reads them from
    jsonl, a memory or broadcast sink from its own store. The event STREAM is an
    interface (EventSink); reconstruction reads whatever the sink yields, never a
    file path."""
    for entry in entries:
        if entry.get("op") == "retract":
            ft = entry["ft"]
            row = _untuple(entry["fact"])
            rows = tuple(t for t in (tuple(r) for r in system._pop_rows(D, ft))
                         if t != row)
            D = _ap(ast.Store(ft), to_lam(rows) if False else _S(to_lam(rows), D))
            continue
        if entry.get("op") == "migrate":
            # a migration BATCH. An OWN-TABLE fact type unions in as one Store
            # write (one derive pass rides the caller's run_rules — the old
            # engine's atomic collection apply is the precedent; the report at
            # migration time carried the verification). An ABSORBED fact type's
            # live population is its RMAP column — a raw cell write would strand
            # the rows outside the columns and break view == reassembly — so its
            # rows BULK-INSTALL the routed 3NF shape (table rows, key index, the
            # ** view cache) in one pass: the same batch precedent, where N
            # validated creates would cost hours on the big apps.
            ft = entry["ft"]
            part = system.rmap_partition(D)
            table = part.get(ft, ft)
            if table != ft:
                D = system.bulk_absorbed_install(D, part, table, ft,
                                                  entry["facts"])
                continue
            rows = {tuple(r) for r in system._pop_rows(D, ft)}
            rows |= {_untuple(f) for f in entry["facts"]}
            D = _ap(ast.Store(ft), _S(to_lam(system._rowsort(rows)), D))
            continue
        D = _ap(_A(2), system.create(D, entry["ft"], to_lam(_untuple(entry["fact"]))))
    return D


def _watermark(D):
    """The snapshot's event-stream watermark: how many stream entries the
    stored D already incorporates. None on a pre-watermark snapshot, written
    when every write passed through this Registry — complete by construction
    at its save time (recompile stamps it)."""
    rows = system._pop_rows(D, "eventWatermark")
    return rows[0][0] if rows and rows[0] else None


def _with_watermark(D, n):
    """Stamp the watermark cell — facts all the way down: how much of the
    stream a snapshot holds is knowledge about the store, so it rides IN the
    store and replaces wholesale like the other meta cells."""
    cells = tuple(c for c in from_lam(D)
                  if not (isinstance(c, tuple) and len(c) >= 2
                          and c[1] == "eventWatermark"))
    return to_lam(cells + (("CELL", "eventWatermark", ((n,),)),))


# ============================ the event sink interface =======================
# The event stream is an INTERFACE, not a file (Samuel, 2026-07-05: the jsonl
# was an undesigned implementation choice). A committed step is APPENDED and the
# stream is READ back for reconstruction, the Connector's two names, and the
# implementation swaps by registration exactly like the rule engines, the
# storage layer, and the UI: the file is one sink, and a broadcast Durable
# Object (the arest tier), a memory buffer, or any backend is another.
class EventSink:
    """append(entry) commits one step to the stream; read() yields the entry
    list for replay_entries. A refused step appends nothing (the caller only
    logs a changed state), so the sink never sees non-events."""

    def append(self, entry):
        raise NotImplementedError

    def read(self):
        raise NotImplementedError


class FileEventSink(EventSink):
    """The default sink: newline-delimited JSON at <app_dir>/<app>.events.jsonl.
    One implementation of the interface; the format is not the design."""

    def __init__(self, app_dir, app_name):
        self.path = os.path.join(app_dir, f"{app_name}.events.jsonl")

    def append(self, entry):
        with open(self.path, "a", encoding="utf-8") as f:
            f.write(json.dumps(entry, ensure_ascii=False) + "\n")

    def read(self):
        return read_log(self.path)


class MemoryEventSink(EventSink):
    """An ephemeral process-global sink, per app name: the event stream with no
    disk (tests, transient apps, and the model for a broadcast sink that keeps
    its own store)."""

    _store = {}

    def __init__(self, app_dir, app_name):
        self.app = app_name
        MemoryEventSink._store.setdefault(app_name, [])

    def append(self, entry):
        MemoryEventSink._store.setdefault(self.app, []).append(entry)

    def read(self):
        return list(MemoryEventSink._store.get(self.app, []))

    @classmethod
    def clear(cls):
        cls._store.clear()


# name -> factory(app_dir, app_name) -> EventSink; register to swap the stream,
# the same discipline as defs.register for the rule-engine overrides
_EVENT_SINKS = {"file": FileEventSink, "memory": MemoryEventSink}


def register_event_sink(name, factory):
    """Bind an event-sink implementation under `name`; a registry selects it by
    name (Registry.event_sink), so swapping the stream is one registration."""
    _EVENT_SINKS[name] = factory


def resolve_event_sink(name, app_dir, app_name):
    """The active sink for an app: the registered factory applied to the app's
    directory and name. Unknown names fall back to the file sink."""
    factory = _EVENT_SINKS.get(name, FileEventSink)
    return factory(app_dir, app_name)


# ============================ the 3NF storage driver =========================
# Storage is a swappable DRIVER (Samuel, 2026-07-05: swappable between sqlite,
# R2, postgresql, clickhouse, anything usable as a 3NF storage driver). The
# interface mirrors arest's own StorageBackend (crates/arest/src/storage.rs),
# the whole-store form over cells: save commits the cell store (arest's commit),
# load rehydrates it (arest's open). SQL backends (sqlite, postgres, clickhouse)
# additionally serve the 3NF relational projection and query the RMAP tables;
# object backends (R2, memory) store cells and skip the SQL surface. The cell
# store is the source of truth; the 3NF is the relational consumer projection.
class StorageDriver:
    sql = False                                               # serves the 3NF query surface?

    def save(self, D):
        raise NotImplementedError

    def load(self):
        raise NotImplementedError                            # -> D (lam) or None if absent

    def exists(self):
        raise NotImplementedError

    def project(self, D):
        """Materialize the 3NF relational projection (SQL backends). An object
        backend has no relational surface and returns None."""
        return None

    def query(self, statement):
        raise ValueError("this storage backend has no SQL surface")


class SqliteStorage(StorageDriver):
    """The default driver: cells one-to-one in a sqlite .db, with the RMAP 3NF
    tables projected beside them (the GraphDL contract). One driver of the
    interface; the backend is not the design."""

    sql = True

    def __init__(self, app_dir, app_name, seal_key=None):
        self.path = os.path.join(app_dir, f"{app_name}.db")
        self.seal_key = seal_key

    def save(self, D):
        save_sqlite(D, self.path, self.seal_key)

    def load(self):
        if not os.path.exists(self.path):
            return None
        return load_sqlite(self.path, self.seal_key)

    def exists(self):
        return os.path.exists(self.path)

    def project(self, D):
        import sqlite3
        from . import ddl
        con = sqlite3.connect(self.path)
        try:
            return ddl.project(D, con)
        finally:
            con.close()

    def query(self, statement):
        import sqlite3
        con = sqlite3.connect(self.path)
        try:
            return con.execute(statement).fetchall()
        finally:
            con.close()


class MemoryStorage(StorageDriver):
    """An ephemeral process-global cell store, per app: storage with no disk
    and no relational surface (tests, transient apps, the model for an R2 or
    Durable Object backend that keeps cells as objects, not 3NF rows)."""

    _store = {}

    def __init__(self, app_dir, app_name):
        self.app = app_name

    def save(self, D):
        MemoryStorage._store[self.app] = _conv(from_lam(D))

    def load(self):
        native = MemoryStorage._store.get(self.app)
        return to_lam(_untuple(native)) if native is not None else None

    def exists(self):
        return self.app in MemoryStorage._store

    @classmethod
    def clear(cls):
        cls._store.clear()


_STORAGE_DRIVERS = {"sqlite": SqliteStorage, "memory": MemoryStorage}


def register_storage_driver(name, factory):
    """Bind a 3NF storage driver under `name`; a registry selects it by name
    (Registry.storage). One registration swaps sqlite for postgres, clickhouse,
    R2, or any 3NF driver."""
    _STORAGE_DRIVERS[name] = factory


def resolve_storage_driver(name, app_dir, app_name):
    factory = _STORAGE_DRIVERS.get(name, SqliteStorage)
    return factory(app_dir, app_name)


# =====================================================================
# Field-level encryption at rest (merged from seal.py, 2026-07-04, the
# fewer-files push: persistence owns what persists sealed). Sensitivity
# DERIVES from data types, the mode from constraints (a keyed role seals
# deterministically because equality must survive sealing; the rest
# randomized), the key scope is the tenant. The cipher here is
# TEST-GRADE (HMAC-SHA256 keystream); production binds real AEAD as a
# boundary def. The engine's part is the derivation and the interface.
# =====================================================================
import base64 as _b64
import hashlib as _hashlib
import hmac as _hmac


SENSITIVE_DATA_TYPES = {"SensitiveText", "Secret", "PII"}
_MARK = "enc1:"


def _data_types(D):
    out = {}
    for r in system._pop_rows(D, "data_type"):
        text = r[0] if r else ""
        if " is " in text:
            name, dt = text.split(" is ", 1)
            out[name.strip()] = dt.strip()
    return out


def seal_plan(D):
    """The derivation: which ⟨fact type, column⟩ seals, in which mode, plus which nouns'
    IDENTIFIERS seal (reference modes on sensitive value types — always deterministic,
    identifiers ARE equality)."""
    dts = _data_types(D)
    sensitive = {vt for vt, dt in dts.items() if dt in SENSITIVE_DATA_TYPES}
    spans = {}
    for r in system._pop_rows(D, "spans"):
        if len(r) == 2:
            spans.setdefault(r[0], set()).add(r[1])
    uc_pos = {}
    for c in system._pop_rows(D, "constraint"):
        if len(c) >= 3 and c[1] in ("uniqueness", "spanning_uniqueness"):
            uc_pos.setdefault(c[2], set()).update(spans.get(c[0], set()))
    roles = {}
    for r in system._pop_rows(D, "role"):
        if len(r) >= 4 and r[3] in sensitive:
            ft, pos = r[1], r[2]
            det = pos in uc_pos.get(ft, set())
            roles[(ft, pos)] = "deterministic" if det else "randomized"
    ids = {}
    for r in system._pop_rows(D, "refMode"):
        if len(r) >= 2 and r[1] in sensitive:
            ids[r[0]] = "deterministic"
    return {"roles": roles, "ids": ids}


# ============================ the cipher boundary (TEST-GRADE) ================
def _stream(key, nonce, n):
    out = b""
    counter = 0
    while len(out) < n:
        out += _hmac.new(key, nonce + counter.to_bytes(4, "big"), _hashlib.sha256).digest()
        counter += 1
    return out[:n]


def seal(key, value, deterministic=False):
    """TEST-GRADE sealing: deterministic mode derives the nonce from the plaintext (same
    value, same ciphertext — equality survives), randomized mode draws it fresh."""
    data = json.dumps(value, ensure_ascii=False).encode("utf-8")
    nonce = (_hmac.new(key, data, _hashlib.sha256).digest()[:8] if deterministic
             else os.urandom(8))
    ct = bytes(a ^ b for a, b in zip(data, _stream(key, nonce, len(data))))
    return _MARK + _b64.b64encode(nonce + ct).decode("ascii")


def unseal(key, token):
    if not (isinstance(token, str) and token.startswith(_MARK)):
        return token
    raw = _b64.b64decode(token[len(_MARK):])
    nonce, ct = raw[:8], raw[8:]
    data = bytes(a ^ b for a, b in zip(ct, _stream(key, nonce, len(ct))))
    return json.loads(data.decode("utf-8"))


def seal_rows(key, rows, cols_modes):
    out = []
    for row in rows:
        row = list(row)
        for (pos, mode) in cols_modes:
            if pos - 1 < len(row):
                row[pos - 1] = seal(key, row[pos - 1], mode == "deterministic")
        out.append(tuple(row))
    return tuple(out)


def unseal_rows(key, rows):
    return tuple(tuple(unseal(key, v) if isinstance(v, str) else v for v in row)
                 for row in rows)

# ===================== ddl =====================
"""The DDL generator: Halpin's Rmap output as SQL (the GraphDL lineage's day job).
The grouping comes from rmap_partition (book 10.3: spanning or absent UC keeps a
fact type its own table, a single-role UC absorbs it into the role-1 player's
table); this module renders it as CREATE TABLE (book 11.12): the reference scheme
as the primary key, absorbed functional fact types as columns with NOT NULL
exactly where a mandatory constraint holds, BOOLEAN columns for absorbed unaries,
per-role REFERENCES to entity tables, and spanning keys on own-table fact types.
Fix-not-inherit: a nullable column REFERENCES without NOT NULL, so an incomplete
entity can never cascade valid rows out of a projection."""
import re

from . import system


def _sql_name(name):
    s = re.sub(r"[^0-9A-Za-z]+", "_", name).strip("_").lower()
    return s or "t"


def _q(name):
    """Every emitted identifier is quoted: the base metamodel projects tables named
    constraint, transition, view — SQL reserved words the old .db also carries."""
    return '"' + name + '"'


def _analyze(D):
    partition = system.rmap_partition(D)
    roles = {}
    for r in system._pop_rows(D, "role"):
        if len(r) >= 4:
            roles.setdefault(r[1], []).append((r[2], r[3]))
    for ft in roles:
        roles[ft].sort()
    ref = {r[0]: r[1] for r in system._pop_rows(D, "refScheme") if len(r) >= 2}
    for r in system._pop_rows(D, "refMode"):                  # Person(.nr): the ref mode
        if len(r) >= 2:
            ref.setdefault(r[0], r[1])
    entities = {r[0] for r in system._pop_rows(D, "instanceOf")
                if len(r) >= 2 and r[1] == "ObjectType"}
    cons = system._pop_rows(D, "constraint")
    mandatory = {}
    for c in cons:
        if len(c) >= 4 and c[1] == "mandatory":
            mandatory.setdefault(c[2], set()).add(c[3])       # ft -> mandated players
    return partition, roles, ref, entities, mandatory


def _key_col(name, ref):
    return f"{_sql_name(name)}_{_sql_name(ref.get(name, 'id'))}"


def _entity_columns(table, partition, roles, ref, entities, entity_tables):
    """The ordered absorbed columns of an entity table: (ft, column, kind, other)
    with kind in unary/value/ref. One naming pass shared by generate and project
    (they must never disagree), deduped with the position suffix the own-table
    branch uses — the base metamodel absorbs two Status roles into transition and
    two Texts into constraint."""
    out, seen = [], {}
    for ft in system.table_columns(partition, table):
        rs = roles.get(ft, [])
        if len(rs) == 1:
            base = _sql_name(ft[len(table):] if ft.startswith(table) else ft)
            kind, other = "unary", None
        else:
            other = next((t for (_p, t) in rs if t != table), None)
            if other in entities and other in entity_tables:
                base, kind = _key_col(other, ref), "ref"
            else:
                base, kind = (_sql_name(other) if other else _sql_name(ft)), "value"
        seen[base] = seen.get(base, 0) + 1
        col = base if seen[base] == 1 else f"{base}_{seen[base]}"
        out.append((ft, col, kind, other))
    return out


def get_view(D, noun, entity_id):
    """CERTIFIED-EQUAL OVERRIDE of DEF("system:entity_view")
    (shared/system.canon) — the host's 3NF per-entity view, kept for
    SPEED only; the canon def is the meaning and test_entity_view pins
    the twin. Reads the SAME store knowledge the canon reads — the
    rmapColumns cell (facts all the way down; a store without the cell
    reads as all-own-table, layout_cells' own reading) — never a
    per-call re-derivation of the partition. Field naming mirrors
    system:sqlcol: unary strips the noun, ref joins the reference mode
    (refScheme over refMode over id), value names the played type, the
    seen-count ordinal suffixes from 2; the ref test is the entity test
    alone (entities ⊆ entity_tables by construction, so the old
    conjunction reduces). Answers ⟨seen, fields, facts⟩ raw;
    Registry.get wraps the app envelope."""
    colrows = [tuple(r) for r in system._pop_rows(D, "rmapColumns")
               if len(r) >= 3 and r[0] == noun]
    absorbed = {r[2] for r in system._pop_rows(D, "rmapColumns")
                if len(r) >= 3}
    roles = {}
    for r in system._pop_rows(D, "role"):
        if len(r) >= 4:
            roles.setdefault(r[1], []).append((r[2], r[3]))
    for ft in roles:
        roles[ft].sort()
    ref = {r[0]: r[1] for r in system._pop_rows(D, "refScheme")
           if len(r) >= 2}
    for r in system._pop_rows(D, "refMode"):
        if len(r) >= 2:
            ref.setdefault(r[0], r[1])
    entities = {r[0] for r in system._pop_rows(D, "instanceOf")
                if len(r) >= 2 and r[1] == "ObjectType"}
    fields, facts, seen = {}, [], False
    counts = {}
    for (_noun, _col, ft) in colrows:
        rs = roles.get(ft, [])
        if len(rs) == 1:
            kind, other = "unary", None
        else:
            other = next((t for (_p, t) in rs if t != noun), None)
            kind = "ref" if other in entities else "value"
        if kind == "unary":
            base = _sql_name(ft[len(noun):] if ft.startswith(noun) else ft)
        elif kind == "ref":
            base = f"{_sql_name(other)}_{_sql_name(ref.get(other, 'id'))}"
        else:
            base = _sql_name(other) if other else _sql_name(ft)
        counts[base] = counts.get(base, 0) + 1
        col = base if counts[base] == 1 else f"{base}_{counts[base]}"
        rows = [tuple(r) for r in system._pop_rows(D, ft)]
        if kind == "unary":
            fields[col] = any(r and r[0] == entity_id for r in rows)
            seen = seen or fields[col]
        else:
            val = {r[0]: r[1] for r in rows if len(r) >= 2}
            fields[other or col] = val.get(entity_id)     # the played type names the field
            seen = seen or entity_id in val
    for r in system._pop_rows(D, "factType"):
        ft = r[0] if len(r) >= 1 else None
        if not ft or ft in absorbed:
            continue
        rs = roles.get(ft, [])
        for row in system._pop_rows(D, ft):
            row = tuple(row)
            if any(p <= len(row) and row[p - 1] == entity_id
                   for (p, player) in rs if player == noun):
                facts.append({"fact_type": ft, "row": list(row)})
                seen = True
    seen = seen or any(r and r[0] == entity_id
                       for r in system._pop_rows(D, noun))
    return seen, fields, facts


def generate(D):
    """{table-or-ft: CREATE TABLE statement}."""
    partition, roles, ref, entities, mandatory = _analyze(D)
    tables = {}
    own = [ft for ft, key in partition.items() if key == ft]
    # every declared entity gets a table (Halpin: entity types with functional
    # roles group them; one without any still anchors its references)
    entity_tables = entities | ({t for t in partition.values()} - set(own))

    for table in sorted(entity_tables):
        cols = [f"    {_q(_key_col(table, ref))} TEXT PRIMARY KEY"]
        for (ft, col, kind, other) in _entity_columns(
                table, partition, roles, ref, entities, entity_tables):
            if kind == "unary":                               # absorbed unary: boolean
                cols.append(f"    {_q(col)} BOOLEAN")
                continue
            # Halpin 11.12: the column hardens only when the MANDATED player is
            # this table (a mandatory on the other role never forces this column)
            null = " NOT NULL" if table in mandatory.get(ft, ()) else ""
            refs = ("" if kind != "ref" else
                    f" REFERENCES {_q(_sql_name(other))}({_q(_key_col(other, ref))})")
            cols.append(f"    {_q(col)} TEXT{null}{refs}")
        tables[table] = (f"CREATE TABLE {_q(_sql_name(table))} (\n"
                         + ",\n".join(cols) + "\n);")

    for ft in sorted(own):
        rs = roles.get(ft, [])
        if not rs:                                            # no roles, no relational shape
            continue
        cols, key, seen = [], [], {}
        for (_pos, player) in rs:
            base = (_key_col(player, ref)
                    if player in entities else _sql_name(player))
            seen[base] = seen.get(base, 0) + 1
            col = base if seen[base] == 1 else f"{base}_{seen[base]}"
            refs = (f" REFERENCES {_q(_sql_name(player))}({_q(_key_col(player, ref))})"
                    if player in entities and player in entity_tables else "")
            cols.append(f"    {_q(col)} TEXT NOT NULL{refs}")
            key.append(col)
        stmt = (f"CREATE TABLE {_q(_sql_name(ft))} (\n" + ",\n".join(cols)
                + f",\n    PRIMARY KEY ({', '.join(_q(c) for c in key)})\n);")
        tables[ft] = stmt
    return tables


def script(D):
    """The whole schema as one executable document, entities before references."""
    return "\n\n".join(generate(D).values())


def project(D, con):
    """Create the schema and POPULATE it from the store. Entity rows are the ids
    playing the entity's roles anywhere (the reference scheme's population,
    derived); absorbed functional fact types fill columns and absorbed unaries fill
    booleans, with an absent value projecting NULL — the row stays (the dangling-FK
    cascade is impossible by construction). Own-table fact types insert row per
    fact. Answers {table: rowcount}."""
    partition, roles, ref, entities, mandatory = _analyze(D)
    own = [ft for ft, key in partition.items() if key == ft]
    entity_tables = entities | ({t for t in partition.values()} - set(own))
    for stmt in generate(D).values():
        # the projection is SOFT where generate is hard (the old engine's
        # projected tables are a data mirror: no NOT NULL beyond the keys), so
        # a migrated population missing a mandatory value lands as a NULL row
        # instead of crashing the compile — visibility over cascade
        stmt = stmt.replace(" TEXT NOT NULL", " TEXT")
        con.execute(stmt.replace("CREATE TABLE", "CREATE TABLE IF NOT EXISTS"))

    def ensure_columns(table, colnames, coltypes):
        # schema evolution on a live db: IF NOT EXISTS never revisits an
        # existing table, so a later compile's new absorbed fact types ALTER
        # in, typed as generate types them (BOOLEAN unaries, TEXT values).
        # Columns the model no longer declares stay behind untouched — the
        # mirror is soft both ways.
        have = {r[1] for r in con.execute(
            f"PRAGMA table_info({_q(_sql_name(table))})")}
        for c in colnames:
            if c not in have:
                con.execute(f"ALTER TABLE {_q(_sql_name(table))} ADD COLUMN "
                            f"{_q(c)} {coltypes.get(c, 'TEXT')}")

    counts = {}
    pops = {}

    def pop(ft):
        if ft not in pops:
            pops[ft] = [tuple(r) for r in system._pop_rows(D, ft)]
        return pops[ft]

    for table in sorted(entity_tables):
        # the derived entity population: every id the entity's roles mention
        ids = set()
        for ft, rs in roles.items():
            for (p, player) in rs:
                if player == table:
                    for row in pop(ft):
                        if len(row) >= p:
                            ids.add(row[p - 1])
        for row in pop(table):                                # plus its own cell
            if row:
                ids.add(row[0])
        colnames = [_key_col(table, ref)]
        coltypes = {}
        per_id = {i: {} for i in ids}
        for (ft, col, kind, _other) in _entity_columns(
                table, partition, roles, ref, entities, entity_tables):
            colnames.append(col)
            coltypes[col] = "BOOLEAN" if kind == "unary" else "TEXT"
            if kind == "unary":
                members = {r[0] for r in pop(ft) if r}
                for i in ids:
                    per_id[i][col] = 1 if i in members else 0
                continue
            val = {r[0]: r[1] for r in pop(ft) if len(r) >= 2}
            for i in ids:
                per_id[i][col] = val.get(i)
        ensure_columns(table, colnames, coltypes)
        marks = ", ".join("?" for _ in colnames)
        for i in sorted(ids):
            con.execute(
                f"INSERT OR REPLACE INTO {_q(_sql_name(table))} "
                f"({', '.join(_q(c) for c in colnames)}) VALUES ({marks})",
                [i] + [per_id[i].get(c) for c in colnames[1:]])
        counts[table] = len(ids)

    for ft in sorted(own):
        rs = roles.get(ft, [])
        if not rs:
            # a fact type with no role rows (a reading over undeclared types) has
            # no relational mapping: named None, never malformed SQL
            counts[ft] = None
            continue
        rows = pop(ft)
        if not rows:
            counts[ft] = 0
            continue
        marks = ", ".join("?" for _ in rs)
        narrow = 0
        for row in rows:
            if len(row) < len(rs):
                # a row narrower than its role count cannot bind; skip it and
                # say so in the count rather than crash the whole projection
                # (messy corpora exist; the population cell keeps the row)
                narrow += 1
                continue
            con.execute(f"INSERT OR REPLACE INTO {_q(_sql_name(ft))} VALUES ({marks})",
                        list(row[:len(rs)]))
        counts[ft] = (len(rows) - narrow if not narrow
                      else {"projected": len(rows) - narrow, "narrow": narrow})
    con.commit()
    return counts

# ===================== migrate =====================
"""Migration from an old-engine app .db (the swap tool). The old store rides the
`cells` table as displayed Objects; two encodings appear in live dbs — the keyed
map '{k=<<Role, value>, ...>>, ...}' and the keyless tuple sequence
'<<<Role, value>, ...>>, <<...>>, ...>' (m:n cells) — with the escape alphabet of
the old ast.rs escape_atom_for_display: a backslash escapes each of \\ < > , { }
= inside atom text. Values may quote structural characters (prose descriptions
carry markup), so parsing MASKS every escaped character into the private-use
plane, scans structure, and proves itself by exact round trip; a cell that fails
the proof is reported, never guessed at.

Populations classify against the NEW model: asserted fact types migrate as BATCH
log entries (one derive pass — the old engine's own atomic collection apply is
the precedent; per-row validated creates would cost hours on the big apps),
derived fact types are never replayed — the engine rederives them and the report
VERIFIES old versus new row sets, which is the migration's parity evidence.
Cells the model does not declare are reported unknown."""
import json
import os
import re
import sqlite3

from . import system

# ---- the cells encoding ----
_MASKED = re.compile(r"\\(.)", re.S)
_UNMASKED = re.compile("\x00(.)", re.S)
# a pair '<Role, value>': role names carry no comma or angle; the value is lazy
# up to a '>' that is followed by another pair, a close, or the end
_PAIR = re.compile(r"<([^,<>]+), (.*?)>(?=, <|>|$)", re.S)


def _mask(s):
    """\\X -> NUL + private-use(X): escaped characters leave the structural
    alphabet entirely (a merely NUL-prefixed '>' would still anchor the scan)."""
    return _MASKED.sub(lambda m: "\x00" + chr(0xE000 + ord(m.group(1))), s)


def _unmask(s):
    return _UNMASKED.sub(lambda m: chr(ord(m.group(1)) - 0xE000), s)


def _parse_pairs(text):
    pairs, pos = [], 0
    for m in _PAIR.finditer(text):
        if m.start() != pos:
            return None
        pairs.append((m.group(1), m.group(2)))
        pos = m.end()
        if text[pos:pos + 2] == ", ":
            pos += 2
    if pos != len(text):
        return None
    return tuple(pairs)


def parse_cell(contents):
    """contents -> [(key-or-None, ((role, value), ...)), ...] with values
    unescaped, or None when the round-trip proof fails."""
    masked = _mask(contents)
    out = _parse_masked(masked)
    if out is None:
        return None
    return [(None if k is None else _unmask(k),
             tuple((_unmask(r), _unmask(v)) for (r, v) in ps))
            for (k, ps) in out]


def _parse_masked(contents):
    s = contents.strip()
    if s.startswith("<<<") and s.endswith(">>>"):
        # the keyless SEQUENCE of tuples: '<T1, T2, ...>', each Ti '<<R, v>, ...>'
        body = s[1:-1]
        entries, pos = [], 0
        for m in re.finditer(r"<(<.*?>)>(?=, <<|$)", body, re.S):
            if m.start() != pos:
                return None
            pairs = _parse_pairs(m.group(1))
            if pairs is None:
                return None
            entries.append((None, pairs))
            pos = m.end()
            if body[pos:pos + 2] == ", ":
                pos += 2
        if pos != len(body):
            return None
        rebuilt = "<" + ", ".join(
            "<" + ", ".join(f"<{r}, {v}>" for (r, v) in ps) + ">"
            for (_k, ps) in entries) + ">"
        return entries if rebuilt == s else None
    if not (s.startswith("{") and s.endswith("}")):
        return None
    inner = s[1:-1]
    if not inner:
        return []
    # the EMPTY population entry: 'key=φ' is the old engine's phi for the
    # empty object and contributes no rows; strip such entries first and
    # run the round-trip proof over the remainder (support.auto.dev's two
    # live cells carry exactly this form)
    inner = re.sub(r"(?:^|(?<=, ))[^=<>{}]+=φ(, |$)", "", inner)
    inner = inner.rstrip(", ").rstrip(",")
    if not inner:
        return []
    starts = [m.start() for m in re.finditer(r"=<<", inner)]
    if not starts:
        return None
    keys, bodies, prev_end = [], [], 0
    for i, st in enumerate(starts):
        key = inner[prev_end:st]
        if i > 0:
            if not key.startswith(", "):
                return None
            key = key[2:]
        keys.append(key)
        body_start = st + len("=<<")
        end = starts[i + 1] if i + 1 < len(starts) else len(inner)
        seg = inner[body_start:end]
        close = seg.rfind(">>")
        if close < 0:
            return None
        bodies.append(seg[:close])
        prev_end = body_start + close + 2
    if prev_end != len(inner):
        return None
    entries = []
    for key, body in zip(keys, bodies):
        pairs = _parse_pairs("<" + body + ">")
        if pairs is None:
            return None
        entries.append((key, pairs))
    rebuilt = "{" + ", ".join(
        k + "=<" + ", ".join(f"<{r}, {v}>" for (r, v) in ps) + ">"
        for (k, ps) in entries) + "}"
    # the proof runs over the phi-reduced inner: stripped empty entries
    # contribute nothing to either side
    return entries if rebuilt == "{" + inner + "}" else None


# ---- classification and replay ----
def read_cells(db_path):
    con = sqlite3.connect(db_path)
    try:
        return dict(con.execute("SELECT name, contents FROM cells").fetchall())
    finally:
        con.close()


# the old engine's REFLECTION layer: its self-description of facts, types,
# roles and readings, materialized as cells (the claude audit: 19,008
# Fact_is_of_Fact_Type rows, 1.2MB — dragged through every derive round when
# migrated). This engine's own compile IS the self-description, so these
# report instead of replay — UNLESS a compiled rule reads one (the base's
# arity rule counts Fact_Type_has_Role), in which case live derivations feed
# on it and it must migrate.
REFLECTION = {
    "Fact_is_of_Fact_Type", "Resource_is_instance_of_Noun",
    "State_Machine_is_instance_of_Noun", "Fact_Type_has_Role",
    "Role_is_used_in_Reading", "Noun_plays_Role", "Reading_has_Text",
    "Fact_Type_has_Reading", "Reading_is_used_by_Verb",
    "Noun_has_Object_Type", "Noun_has_World_Assumption",
    "Function_belongs_to_Domain", "Resource_is_of_Function",
    "Resource_belongs_to_Domain",
}


def plan(D, cells):
    """Classify the old cells against the compiled model: asserted (rows to
    migrate, in role order), derived-with-a-rule (kept aside for verification —
    the engine rederives), STORED STATE (marked derived but NO rule derives it:
    the old base's own comments record the engine's imperative writers owning
    such cells, e.g. State_Machine_is_for_Resource after its underspecified rule
    was removed — these migrate as data), unknown and unparsed (reported)."""
    fts = {f[0] for f in system._pop_rows(D, "factType") if f}
    kinds = {r[0]: r[1] for r in system._pop_rows(D, "derivation")
             if len(r) >= 2}
    ruled = {r[1] for r in system._pop_rows(D, "ruleDerives") if len(r) >= 2}
    rule_read = {r[1] for r in system._pop_rows(D, "ruleReads") if len(r) >= 2}
    out = {"asserted": {}, "derived": {}, "stored_state": [],
           "reflection": [], "unknown": [], "unparsed": []}
    for name, contents in cells.items():
        parsed = parse_cell(contents or "{}")
        if parsed is None:
            out["unparsed"].append(name)
            continue
        if name in REFLECTION:
            # unconditional after proposal B (2026-07-04): the instance
            # mirror derives engine-side whenever a rule reads it, so no
            # reflection cell ever migrates
            out["reflection"].append(name)
            continue
        rows = [tuple(v for (_r, v) in ps) for (_k, ps) in parsed]
        if kinds.get(name) == "fully-derived" and name in ruled:
            # a PURE derivation: the engine rederives it; the report verifies
            out["derived"][name] = rows
        elif name in fts or name in kinds:
            # asserted, or derived-and-stored/semiderived/ruled-but-plain: the
            # old engine's imperative writers own such populations — data
            out["asserted"][name] = rows
            if name in kinds or name in ruled:
                out["stored_state"].append(name)
        else:
            out["unknown"].append(name)
    return out


def _prose_like(value):
    """Sentence-shaped content where an atomic value belongs: several words
    with sentence punctuation, or outright paragraph length. The heuristic is
    deliberately conservative — the audit flags for re-authoring at swap time,
    it never blocks."""
    if not isinstance(value, str):
        return False
    words = value.split()
    if len(value) > 160:
        return True
    return len(words) >= 6 and any(m in value for m in (". ", "; ", ", "))


def audit_authoring(plan_out, D=None):
    """The mis-authoring audit: prose crammed into VALUES (catch-all text
    fields), prose used as IDS (the first role of a row is the reference; a
    sentence there is an authoring defect), and prose ENUM MEMBERS in the
    readings' possible-values declarations. Answers {cell, kind, count,
    sample} findings — the swap-time cleanup list, never a block."""
    findings = []
    for ft, rows in sorted(plan_out["asserted"].items()):
        hits_v = [r for r in rows if any(_prose_like(v) for v in r[1:])]
        hits_i = [r for r in rows if r and isinstance(r[0], str)
                  and len(r[0].split()) >= 5]
        if hits_v:
            findings.append({"cell": ft, "kind": "prose_value",
                             "count": len(hits_v),
                             "sample": str(hits_v[0])[:160]})
        if hits_i:
            findings.append({"cell": ft, "kind": "prose_id",
                             "count": len(hits_i),
                             "sample": str(hits_i[0][0])[:160]})
    if D is not None:
        for r in system._pop_rows(D, "valueConstraint"):
            if len(r) >= 2 and isinstance(r[1], str):
                members = re.findall(r"'([^']*)'", r[1])
                bad = [m for m in members
                       if _prose_like(m) or len(m.split()) >= 6]
                if bad:
                    findings.append({"cell": r[0], "kind": "prose_enum",
                                     "count": len(bad),
                                     "sample": bad[0][:160]})
    return findings


def status_bridge(D, cells):
    """The 0.9.0 status mapping (adopt-new, arest docs/0.9.0-status-interop.md):
    the old engine keys machine status by State Machine instance and projects
    it per resource; the new engine's status(e) is the per-noun "is currently
    in Status" fact type on the governed noun's RMAP column. The old CURRENT
    state — the fold's value, Prop. onestep's s — migrates as data: each old
    ⟨resource, status⟩ routes to the governed noun whose population carries the
    id, membership read by ROLE OCCURRENCE (an m:n-only noun never lands in a
    table index). Source: the old per-resource projection cell when present,
    else the SM-keyed cell joined through State_Machine_is_for_Resource.
    → (routed {new_ft: rows}, unrouted ids)."""
    from . import system
    markers = [tuple(r)[:2] for r in system._pop_rows(D, "smStatusFt")
               if len(r) >= 2]
    if not markers:
        return {}, []

    def rows_of(name):
        parsed = parse_cell(cells.get(name) or "{}")
        return [tuple(v for (_r, v) in ps) for (_k, ps) in (parsed or [])]

    old = [r for r in rows_of("Resource_is_currently_in_Status") if len(r) >= 2]
    if not old:
        for_map = {r[0]: r[1] for r in rows_of("State_Machine_is_for_Resource")
                   if len(r) >= 2}
        old = [(for_map[r[0]], r[1])
               for r in rows_of("State_Machine_is_currently_in_Status")
               if len(r) >= 2 and r[0] in for_map]
    if not old:
        return {}, []
    pops = {}
    for (noun, _ft) in markers:
        pop = {row[0] for row in system._pop_rows(D, noun) if row}
        for r in system._pop_rows(D, "role"):
            if len(r) >= 4 and r[3] == noun:
                pos = r[2]
                for frow in system._pop_rows(D, r[1]):
                    if len(frow) >= pos:
                        pop.add(frow[pos - 1])
        pops[noun] = pop
    routed, unrouted = {}, []
    for (rid, status) in old:
        ft = next((ft for (noun, ft) in markers if rid in pops[noun]), None)
        if ft is None:
            unrouted.append(rid)
        else:
            routed.setdefault(ft, []).append((rid, status))
    return routed, unrouted


def replay_into(registry, app, old_db):
    """Migrate an old .db's asserted populations into the app as BATCH log
    entries, recompile (the log replays through the same path every compile
    after), and answer the report — including the derived-population
    verification, old engine versus this one, and the status bridge (the old
    CURRENT machine state landing on the new per-noun status columns)."""
    if os.path.abspath(old_db) == os.path.abspath(registry._db(app)):
        raise ValueError("old_db is the app's own database: compiling would "
                         "overwrite the source cells before they are read — "
                         "pass a snapshot copy")
    registry.compile(app)
    D = registry._load(app)
    cells = read_cells(old_db)
    p = plan(D, cells)
    sink = registry._sink(app)                                # the event stream, not a file
    for ft, rows in sorted(p["asserted"].items()):
        if rows:
            sink.append({"op": "migrate", "ft": ft,
                         "facts": [list(r) for r in rows]})
    registry.compile(app)
    D = registry._load(app)
    # the status bridge routes against the POST-replay populations — the
    # entities it routes to may themselves have just migrated
    bridge, unrouted = status_bridge(D, cells)
    if bridge:
        for ft, rows in sorted(bridge.items()):
            sink.append({"op": "migrate", "ft": ft,
                         "facts": [list(r) for r in rows]})
        registry.compile(app)
        D = registry._load(app)
    verify = {}
    part = system.rmap_partition(D)
    for ft, rows in sorted(bridge.items()):
        new_rows = {tuple(str(x) for x in r)
                    for r in system.ft_view(D, ft, part)}
        old_set = {tuple(str(x) for x in r) for r in rows}
        verify[ft] = {"old": len(old_set), "new": len(new_rows),
                      "match": old_set <= new_rows,
                      "missing": sorted(old_set - new_rows)[:5],
                      "extra": sorted(new_rows - old_set)[:5]}
    for ft, old_rows in sorted(p["derived"].items()):
        # compare as STRINGS: the old cells serialize every value as text, and
        # an aggregate this engine derives is a number (Fact_Type_has_Arity)
        new_rows = {tuple(str(x) for x in r) for r in system._pop_rows(D, ft)}
        old_set = {tuple(str(x) for x in r) for r in old_rows}
        verify[ft] = {"old": len(old_set), "new": len(new_rows),
                      "match": old_set == new_rows,
                      "missing": sorted(old_set - new_rows)[:5],
                      "extra": sorted(new_rows - old_set)[:5]}
    return {"migrated": {ft: len(rows) for ft, rows in p["asserted"].items()
                         if rows},
            "stored_state": sorted(p["stored_state"]),
            "reflection": sorted(p["reflection"]),
            "verify": verify, "unknown": sorted(p["unknown"]),
            "unparsed": sorted(p["unparsed"]),
            "status_bridge": {ft: len(rows) for ft, rows in bridge.items()},
            "status_unrouted": sorted(set(unrouted))[:20],
            "authoring": audit_authoring(p, D)}

# ===================== federate =====================
"""External federation (the platform arc): fetch-and-store through the same front door.
httpFetch is the paper's NAMED binding — a host fetch the caller may override with a
fixture twin (the universal-interface principle applied at the module edge). The
importer VERBALIZES the external vocabulary into canonical FORML readings
(verbalize-then-ingest, never a second metamodel): namespaced nouns carry the source
prefix (schema:Product — the grammar already parses them), classes become entity types,
object properties become fact types, datatype properties become value types with
has-readings, items become instance facts, and provenance lands as federatedFrom rows.
Refetch is idempotent by set semantics."""
import json

from . import ast, forml, meta, system
from .lam import to_lam
from .reduce import apply as _ap
import pyarest.lam as L


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _local(iri):
    return iri.split(":", 1)[1] if ":" in iri else iri


def _http_fetch_impl(mu):
    """The paper's NAMED binding, registered into DEFS ("a server registers httpFetch
    and upsert"): ATOM(url) -> ATOM(parsed payload). Tests re-register this name with a
    fixture twin — DEFS is the DI container, swapping is re-registering."""
    from urllib.request import urlopen, Request
    from . import defs as _d

    def g(o):
        url = _d._aval(o)
        if not isinstance(url, str):
            return L.BOT
        req = Request(url, headers={"User-Agent": "pyarest-federation/0.1"})
        with urlopen(req, timeout=60) as r:
            return L.atom(json.loads(r.read().decode("utf-8")))
    return g


def _translator_impl(fn):
    """Wrap a verbalizer (payload -> readings text) as a registered def: ATOM(payload)
    -> ATOM(readings)."""
    from . import defs as _d

    def impl(mu):
        def g(o):
            payload = _d._aval(o)
            if payload is None:
                return L.BOT
            vocab = payload.get("vocab", payload) if isinstance(payload, dict) else payload
            readings = fn(vocab)
            if isinstance(payload, dict) and payload.get("items"):
                readings += jsonld_items_to_readings(payload["items"], vocab)
            return L.atom(readings)
        return g
    return impl


def register_bindings():
    """Register the federation names into DEFS. Idempotent; call again to restore the
    real bindings after a test swapped them."""
    from .defs import register
    register("httpFetch", _http_fetch_impl)
    register("translate_jsonld", _translator_impl(jsonld_to_readings))
    register("translate_gs1", _translator_impl(gs1_to_readings))
    register("translate_onet", _translator_impl(onet_to_readings))


# ============================ schema.org (JSON-LD) ============================
def _ids(x):
    """domainIncludes/rangeIncludes/subClassOf come as a dict OR a list of dicts in the
    live feed; normalize to a list of ids."""
    if x is None:
        return []
    if isinstance(x, dict):
        return [x.get("@id")]
    return [e.get("@id") for e in x if isinstance(e, dict)]


def _types(node):
    t = node.get("@type")
    return t if isinstance(t, list) else ([t] if t else [])


def _vocab_shape(vocab):
    classes, props, subclass = [], [], []
    for node in vocab.get("@graph", []):
        ts = _types(node)
        if "rdfs:Class" in ts and "schema:DataType" not in ts:
            classes.append(node["@id"])
            for sup in _ids(node.get("rdfs:subClassOf")):
                if sup:
                    subclass.append((node["@id"], sup))
        elif "rdf:Property" in ts:
            props.append(node)
    return classes, props, subclass


def jsonld_to_readings(vocab):
    """schema.org-style JSON-LD (@graph of rdfs:Class / rdf:Property with possibly
    LIST-valued domainIncludes / rangeIncludes, and rdfs:subClassOf) verbalized as
    canonical FORML — subclass links become the ordinary subtype reading."""
    classes, props, subclass = _vocab_shape(vocab)
    cset = set(classes)
    out = [f"{c} is an entity type." for c in classes]
    out += [f"{sub} is a subtype of {sup}." for (sub, sup) in subclass if sup in cset]
    declared_vts = set()
    for p in props:
        rngs = _ids(p.get("schema:rangeIncludes"))
        for dom in _ids(p.get("schema:domainIncludes")):
            if dom not in cset:
                continue
            obj_rngs = [r for r in rngs if r in cset]
            if obj_rngs:
                out.append(f"{dom} {_local(p['@id'])} {obj_rngs[0]}.")
            elif rngs:
                if p["@id"] not in declared_vts:
                    declared_vts.add(p["@id"])
                    out.append(f"{p['@id']} is a value type.")
                    # the range is DECLARED (schema:Text, schema:Number, schema:Boolean,
                    # …): transfer it as the value type's Data Type — no guessing, and
                    # the sealing/type derivations read it like any other declaration
                    out.append(f"Data Type: {p['@id']} is {rngs[0]}.")
                out.append(f"{dom} has {p['@id']}.")
    return "\n".join(out) + "\n"


def jsonld_items_to_readings(items, vocab):
    """Instance items → quoted instance-fact readings through the SAME grammar."""
    classes, props, _sub = _vocab_shape(vocab)
    cset = set(classes)
    bykey = {p["@id"]: p for p in props}
    out = []
    for item in items:
        cls = item.get("@type")
        iid = item.get("@id")
        for key, val in item.items():
            if key.startswith("@") or key not in bykey:
                continue
            rngs = _ids(bykey[key].get("schema:rangeIncludes"))
            obj_rngs = [r for r in rngs if r in cset]
            if obj_rngs:
                out.append(f"{cls} '{iid}' {_local(key)} {obj_rngs[0]} '{val}'.")
            else:
                out.append(f"{cls} '{iid}' has {key} '{val}'.")
    return "\n".join(out) + ("\n" if out else "")


def _lval(v):
    """A JSON-LD literal: a bare string or {"@language","@value"}."""
    if isinstance(v, dict):
        return v.get("@value")
    return v if isinstance(v, str) else None


def jsonld_instance_graph_to_readings(data, types=("schema:Article",), keep=None):
    """A wild JSON-LD instance graph (Wikidata's EntityData shape: typed nodes with
    compact schema.org keys) → declarations + quoted instance-fact readings, through
    the same grammar. `keep` optionally filters nodes (e.g. by language)."""
    nodes = [n for n in data.get("@graph", []) if n.get("@type") in types]
    if keep:
        nodes = [n for n in nodes if keep(n)]
    used = []
    for n in nodes:
        for k in n:
            if not k.startswith("@") and k not in used and _lval(n[k]) is not None:
                used.append(k)
    out = [f"{t} is an entity type." for t in types]
    for k in used:
        out.append(f"schema:{k} is a value type.")
    for t in types:
        for k in used:
            out.append(f"{t} has schema:{k}.")
    for n in nodes:
        nid = n.get("@id")
        t = n.get("@type")
        for k in used:
            v = _lval(n.get(k))
            if v is not None:
                out.append(f"{t} '{nid}' has schema:{k} '{v}'.")
    return "\n".join(out) + "\n"


# ============================ GS1 (GPC bricks) and O*NET ======================
def gs1_to_readings(gpc):
    out = ["gs1:Brick is an entity type.", "gs1:Title is a value type.",
           "gs1:Brick has gs1:Title."]
    for b in gpc.get("bricks", []):
        out.append(f"gs1:Brick '{b['code']}' has gs1:Title '{b['title']}'.")
    return "\n".join(out) + "\n"


def onet_to_readings(onet):
    out = ["onet:Occupation is an entity type.", "onet:Title is a value type.",
           "onet:Occupation has onet:Title."]
    for o in onet.get("occupations", []):
        out.append(f"onet:Occupation '{o['code']}' has onet:Title '{o['title']}'.")
    return "\n".join(out) + "\n"


# ============================ fetch-and-store =================================
def fetch_and_store(D, url, fetch=None):
    """Pull the external vocabulary and items, verbalize, ingest through compile_model
    (the same front door as every reading), and record provenance. Returns (D, report).
    With fetch=None the fetch resolves through DEFS by name (rho of httpFetch) — DEFS
    is the DI container; the M-declared entry is fetch_source."""
    if fetch is None:
        from . import defs as _d
        from .reduce import apply as _apply
        from .lam import atom as _A
        payload = _d._aval(_apply(_A("httpFetch"), _A(url)))
    else:
        payload = fetch(url)
    vocab = payload.get("vocab", payload)
    readings = jsonld_to_readings(vocab) + \
        jsonld_items_to_readings(payload.get("items", []), vocab)
    if D is None:
        D = meta.initial_D()
    D, rep = forml.compile_model(readings, D)
    classes, _props, _sub = _vocab_shape(vocab)
    rows = {tuple(r) for r in system._pop_rows(D, "federatedFrom")} | \
        {(c, url) for c in classes}
    D = _ap(ast.Store("federatedFrom"), _S(to_lam(tuple(sorted(rows))), D))
    return D, rep


# ============================ M-declared sources ==============================
MODULE = None


def _module_readings():
    global MODULE
    if MODULE is None:
        from . import canon as paths
        MODULE = open(paths.shared("federation.md"), encoding="utf-8").read()
    return MODULE


def _lookup(D, ft, key):
    for r in system._pop_rows(D, ft):
        if len(r) >= 2 and r[0] == key:
            return r[1]
    return None


def fetch_source(D, source):
    """THE federation entry, everything read off M and resolved through DEFS: the
    Source's Url, its Connector, the Connector's Fetcher and Translator (definition
    NAMES — rho resolves them, so any backend is a Connector declaring two names).
    fetch -> translate -> ingest through compile_model -> provenance."""
    from . import defs as _d
    from .reduce import apply as _apply
    from .lam import atom as _A, from_lam
    url = _lookup(D, "Source_has_Url", source)
    conn = _lookup(D, "Source_uses_Connector", source)
    fetcher = _lookup(D, "Connector_fetches_with_Fetcher", conn)
    translator = _lookup(D, "Connector_translates_with_Translator", conn)
    if not all((url, conn, fetcher, translator)):
        return D, {"unparsed": [f"source {source!r} is not fully declared"]}
    with _d.step(D):
        payload = _apply(_A(fetcher), _A(url))
        readings = from_lam(_apply(_A(translator), payload))
    if not isinstance(readings, str):
        return D, {"unparsed": [f"translator {translator!r} answered no readings"]}
    D, rep = forml.compile_model(readings, D)
    rows = {tuple(r) for r in system._pop_rows(D, "federatedFrom")} | {(source, url)}
    D = _ap(ast.Store("federatedFrom"), _S(to_lam(tuple(sorted(rows))), D))
    return D, rep


register_bindings()

# ===================== apps =====================
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

from . import forml, system

_MARKER = ".pyarest-active-app"


def default_base():
    """The vendored base readings directory (shared/base), or None if absent."""
    from . import canon as paths
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
        self.event_sink = "file"                              # the active sink name; swap it
        self.storage = "sqlite"                               # the active 3NF driver; swap it

    # ---- inventory ----
    def _app_dir(self, name):
        return os.path.join(self.root, name)

    def _sink(self, name):
        """The app's event sink, resolved by the active name (default file):
        the event stream is an interface, so a registry can serve a broadcast
        or memory sink instead by setting event_sink."""
        return persist.resolve_event_sink(self.event_sink, self._app_dir(name), name)

    def _storage(self, name):
        """The app's storage driver, resolved by the active name (default
        sqlite): a swappable 3NF driver, so a registry can serve postgres,
        clickhouse, R2, or memory instead by setting storage."""
        return persist.resolve_storage_driver(self.storage, self._app_dir(name), name)

    def _db(self, name):
        return os.path.join(self._app_dir(name), f"{name}.db")

    def _readings(self, name):
        """The app's reading files, the old engine's walk (rebuild.rs
        load_app_readings): RECURSIVE over readings/, app.md first, then
        depth-then-name — instance slices live in subdirectories (the claude
        app's readings/instances/), and core nouns must be in context first."""
        d = os.path.join(self._app_dir(name), "readings")
        if not os.path.isdir(d):
            return []
        found = []
        for root, _dirs, files in os.walk(d):
            for fn in files:
                if fn.endswith(".md"):
                    found.append(os.path.join(root, fn))
        found.sort(key=lambda p: (len(os.path.relpath(p, d).split(os.sep)), p))
        app_md = [p for p in found if os.path.basename(p) == "app.md"
                  and os.path.dirname(p) == d]
        return app_md + [p for p in found if p not in app_md]

    def list(self):
        out = []
        for name in sorted(os.listdir(self.root)):
            d = self._app_dir(name)
            if not os.path.isdir(d) or not os.path.isdir(os.path.join(d, "readings")):
                continue
            db = self._db(name)                               # mtime is a file nicety
            out.append({
                "name": name,
                "root": d,
                "compiled": self._storage(name).exists(),     # the driver, not a file
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
        # status(e) is an ORM fact type ("<Noun> is currently in Status"), so RMAP
        # absorbs it as a column and the machine reads/overwrites it there; wired
        # before replay so the machine fires into the column, not the noun_status wart
        D = system.status_facts(D)
        # the event stream replays through the SAME create (facts are the source
        # of truth; the .db is disposable, set semantics make replay idempotent),
        # read from whatever sink the registry holds, never a file path
        entries = self._sink(name).read()
        if entries:
            D = persist.replay_entries(D, entries)
            D = system.run_rules(D)
        # readings- AND log-carried machine events fold ONCE, after every
        # event fact is present and the post-replay derive has run (the
        # started-backfill implications are derivation consequents the fold
        # consumes); the machine itself orders the walk. The derive after
        # projects the folded column onto its derived heads (the Task has
        # Task Status class) before the snapshot — and BOTH skip when the
        # fold changed nothing (identity answer: no machines, no events,
        # nothing to init — the common probe-app case pays one fixpoint,
        # not three; the fold-derive-cost lever's cheap half)
        D2 = system.machine_fold(D)
        if D2 is not D:
            D = system.run_rules(D2)
        # the snapshot records how much of the stream it holds, so a load can
        # replay exactly the tail another host appended after this save
        D = persist._with_watermark(D, len(entries))
        D = system.layout_cells(D)
        D = system.scheduler_cells(D)
        D = system.generator_cells(D)
        D = system.create_handlers(D)                         # create:<ft> defs, native apply
        drv = self._storage(name)
        drv.save(D)                                           # the cell store, through the driver
        self._sidecar(name, D)
        # the RMAP 3NF projection rides with a SQL backend (the GraphDL
        # contract: the relational tables downstream consumers read); an object
        # backend has no relational surface and skips it
        if drv.sql:
            rep["projected"] = drv.project(D)
        rep["app"] = name
        return rep

    def _sidecar(self, name, D):
        """<name>.store.json beside the .db: one serve-protocol line persisted
        (set_store's payload — d, process, overrides, cases), the Rust
        resident's boot food. The resident feeds the file through the same
        ingestion path a --serve line takes, so writing it at every snapshot
        site keeps the sidecar and the .db in lockstep by construction."""
        from .lam import from_lam
        from .polyglot import _conv
        from . import defs as _defs
        process = [[n, _conv(from_lam(obj))]
                   for n, (kind, obj) in _defs.latest.items()
                   if kind == "compiled"]
        payload = {"d": _conv(from_lam(D)), "process": process,
                   "overrides": 1, "cases": []}
        path = os.path.join(self._app_dir(name), f"{name}.store.json")
        # the tmp name carries the pid: the Rust resident writes this sidecar
        # too, and two writers sharing one tmp path tear the file mid-stream
        tmp = f"{path}.{os.getpid()}.tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(payload, f, ensure_ascii=False)
        os.replace(tmp, path)

    # ---- the write side: eq. create against the app's store ----
    def apply(self, name, fact_type, fact):
        """One create against the app's store: validate over the derived candidate,
        commit iff no alethic violation (eq. create), append the committed step to
        the event log (a refusal appends nothing), snapshot, and answer the RECEIPT:
        committed, the violation set, and the representation parts."""
        from .lam import to_lam, from_lam, atom as _A
        from .reduce import apply as _ap
        row = tuple(fact)
        # THE ID-SENTINEL GUARD (the phi phantom, 2026-07-08: an empty id
        # leaked as the empty-store atom through an old write and its
        # ragged wide row bottomed every absorbed fetch of the table).
        # A key position carrying the phi atom or the empty string is
        # never a modeling intent — refuse before any evaluation. Replay
        # stays ungated: the log is history, and retract must still
        # reach such rows to clean them.
        if row and isinstance(row[0], str) and row[0] in ("", "φ"):
            receipt = {"app": name, "fact_type": fact_type,
                       "fact": list(row), "committed": False,
                       "violations": [["id-sentinel",
                                       "a key must not be empty or the "
                                       "phi atom"]]}
            self.last_receipt = receipt
            return receipt
        D = self._load(name)
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
            self._sink(name).append({"ft": fact_type, "fact": list(row)})
            wm = persist._watermark(D2)
            if wm is not None:
                # our append joins the count, so the saved snapshot holds
                # exactly the stream it claims to
                D2 = persist._with_watermark(D2, wm + 1)
            self._storage(name).save(D2)
            self._sidecar(name, D2)
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
        self._sink(name).append({"op": "retract", "ft": fact_type,
                                 "fact": list(row)})
        self.compile(name)                                    # rebuild: log applied
        receipt = {"app": name, "fact_type": fact_type, "fact": list(row),
                   "committed": True, "violations": []}
        self.last_receipt = receipt
        return receipt

    # ---- reads ----
    def _load(self, name):
        drv = self._storage(name)
        D = drv.load()
        if D is None:
            raise FileNotFoundError(f"app {name!r} is not compiled (no store)")
        # the STREAM is the store of record and the snapshot is disposable, so
        # a snapshot TRAILING the stream heals here: the Rust resident commits
        # natively by appending to the stream (it never writes the .db), and
        # the tail beyond the snapshot's watermark replays through the same
        # create. The healed watermark rides the returned store, so a
        # committing caller persists it. A pre-watermark snapshot has no count
        # to trail from and loads as-is; recompile stamps it.
        wm = persist._watermark(D)
        if wm is not None:
            entries = self._sink(name).read()
            if len(entries) > wm:
                tail = entries[wm:]
                D = persist.replay_entries(D, tail)
                D = system.run_rules(D, changed=sorted(
                    {e["ft"] for e in tail if e.get("ft")}))
                D = persist._with_watermark(D, len(entries))
        return D

    def query(self, name, fact_type):
        D = self._load(name)
        # a machine-managed status fact type is advanced in place on its RMAP column
        # (row_overwrite), leaving the append log with the create events; its live
        # population is therefore the column (ft_view). User-managed fact types read
        # the log, which create AND retract keep current.
        if fact_type in {r[1] for r in system._pop_rows(D, "smStatusFt") if len(r) >= 2}:
            return sorted(system.ft_view(D, fact_type, system.rmap_partition(D)))
        return [tuple(r) for r in system._pop_rows(D, fact_type)]

    def entities(self, name, noun):
        """The noun's population for the UI containers: its own table's
        keys unioned with the role-1 keys of every fact type it heads —
        a fresh compile carries pops before any table cell
        materializes, and an entity is an entity by playing a fact."""
        D = self._load(name)
        keys = {str(r[0]) for r in system._pop_rows(D, noun) if r}
        for r in system._pop_rows(D, "role"):
            if len(r) >= 4 and r[2] == 1 and r[3] == noun:
                keys |= {str(x[0])
                         for x in system._pop_rows(D, r[1]) if x}
        keys -= {"", "φ"}
        return sorted(keys)

    def items(self, name, noun):
        """The list perspective's rows as ⟨id, text, value⟩ — the
        ContentCell bindings: text from the noun's first binary fact
        type population (one pop read), value from the machine column
        when governed. One home; both desktop containers read it."""
        D = self._load(name)
        ids = self.entities(name, noun)
        text_ft = next(
            (r[1] for r in sorted(system._pop_rows(D, "role"))
             if len(r) >= 4 and r[2] == 1 and r[3] == noun
             and any(len(q) >= 3 and q[1] == r[1] and q[2] == 2
                     for q in system._pop_rows(D, "role"))), None)
        texts = {}
        if text_ft:
            for r in system._pop_rows(D, text_ft):
                if len(r) >= 2 and str(r[0]) not in texts:
                    texts[str(r[0])] = str(r[1])
        status = dict(system._status_rows(D, noun))
        return [[i, texts.get(i, i), status.get(i, "")] for i in ids]

    def sql(self, name, statement):
        return self._storage(name).query(statement)          # the 3NF surface (SQL backends)

    def get(self, name, noun, entity_id):
        """The 3NF per-entity view envelope; the view itself is get_view,
        the certified-equal override of the canon definition."""
        from . import ddl
        D = self._load(name)
        seen, fields, facts = ddl.get_view(D, noun, entity_id)
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

    def synthesize(self, name, id, noun=None):
        """The old engine's synthesize verb, engine half: the entity's facts
        paired with their fact types' reading templates, post-derive (the
        canonical system:verbalize), plus a plain rendering per pair. The
        engine guarantees content; wording is the caller's concern (an LLM
        shapes it, or the rendering below stands)."""
        from .reduce import apply as _apply
        from .kernel import atom as A, from_lam
        D = self._load(name)
        got = from_lam(_apply(_apply(A("system:verbalize"), A(id)), D))
        facts = []
        for r in got if isinstance(got, tuple) else ():
            if not (isinstance(r, tuple) and len(r) == 2):
                continue
            reading, row = str(r[0]), tuple(r[1])
            try:
                text = reading.format(*row)
            except (IndexError, KeyError):
                text = reading
            facts.append({"reading": reading, "row": list(row),
                          "text": text})
        return {"app": name, "id": id, "facts": facts}

    @staticmethod
    def _pair(head, row):
        from .kernel import atom as A, to_lam
        import pyarest.lam as L
        l = L.CONS(to_lam(tuple(row)))(L.NIL)
        return L.SEQ(L.CONS(A(head))(l))

    def explain(self, name, id, fact=None):
        """The old engine's explain verb: the derivation chain for the
        entity's facts (which rules fired, supporting which rows, reading
        which cells; GMS93's derivation notion made queryable) plus the audit
        trail from the events journal. Host-walked over the rule M-facts for
        now; the canonical core follows the quasiquote pattern (building the
        filter predicate tree at run with the rule id embedded)."""
        from .reduce import apply as _apply
        from .kernel import atom as A, from_lam
        import json
        import os
        D = self._load(name)
        from . import system as _sys
        derives = [tuple(r) for r in _sys._pop_rows(D, "ruleDerives")
                   if len(r) >= 2]
        reads = {}
        for r in _sys._pop_rows(D, "ruleReads"):
            if len(r) >= 2:
                reads.setdefault(r[0], []).append(r[1])
        chains = []
        from . import defs as _defs
        for (rid, head) in derives:
            if fact is not None and head != fact:
                continue
            rows = []
            try:
                with _defs.step(D):                          # rho: the rule
                    out = from_lam(_apply(A(rid), D))        # lives in D's DEFS
                if isinstance(out, tuple):
                    rows = [tuple(x) for x in out if isinstance(x, tuple)]
            except Exception:
                rows = []
            mine = [list(x) for x in rows if id in x]
            if mine or (fact is not None and head == fact):
                entry = {"rule": rid, "head": head, "supports": mine,
                         "reads": sorted(reads.get(rid, []))}
                if mine:
                    # the canonical chain corroborates the host walk (the
                    # same computation any host performs over the canon)
                    try:
                        with _defs.step(D):
                            c = from_lam(_apply(_apply(
                                A("system:explain"),
                                self._pair(head, mine[0])), D))
                        entry["canonical"] = [
                            {"rule": x[0], "fired": x[1] == "T",
                             "reads": sorted(x[2])} for x in c]
                    except Exception:
                        pass
                chains.append(entry)
        trail = []
        for e in self._sink(name).read():                     # the stream, not a file
            if id in json.dumps(e, ensure_ascii=False):
                trail.append(e)
        return {"app": name, "id": id, "chains": chains,
                "audit": trail[-20:]}

    def validate(self, name):
        """The old engine's validate verb: the app's constraint validation over
        the SETTLED store. eq. create judges candidates, so a write never lands
        an alethic offender — but instance facts in readings ingest unvalidated
        and deontic violations commit by design, so a compiled store can carry
        drift. This walks the declared fact types, applies each one's validate
        (forml.validate_for — the same object create runs) to ⟨P, D⟩, and
        reports the non-empty violation sets. An empty list is a clean bill."""
        from .lam import to_lam, from_lam
        from .reduce import apply as _ap
        from . import defs
        import pyarest.lam as L
        D = self._load(name)
        partition = system.rmap_partition(D)
        # constraint kinds by scoped fact type: rows are (cid, kind, …scope…,
        # modality) — the scope columns name fact types (the exclusion family
        # scopes a clause TUPLE), the tail column is the modality
        kinds = {}
        for c in system._pop_rows(D, "constraint"):
            if len(c) < 3:
                continue
            scope = c[2:-1] if c[-1] in ("alethic", "deontic") else c[2:]
            for part in scope:
                for t in (part if isinstance(part, tuple) else (part,)):
                    if isinstance(t, str):
                        kinds.setdefault(t, set()).add(c[1])
        violations = []
        for f in system._pop_rows(D, "factType"):
            if not f:
                continue
            ft = f[0]
            val = forml.validate_for(ft, D, partition)
            pop = tuple(tuple(r) for r in system._pop_rows(D, ft))
            pair = L.SEQ(L.CONS(to_lam(pop))(L.CONS(D)(L.NIL)))
            with defs.step(D):
                _p, v, flag = from_lam(_ap(val, pair))
            if v:
                violations.append(
                    {"fact_type": ft,
                     "kinds": sorted(kinds.get(ft, ())),
                     "offenders": [list(x) if isinstance(x, tuple) else [x]
                                   for x in v],
                     "alethic": flag == "T"})
        return {"app": name, "violations": violations}

    def verify(self, name):
        """CERTIFIED-EQUAL OVERRIDE of DEF("system:verify_store")
        (shared/system.canon, 2026-07-08 — the canonicalization arc's
        last host-only meaning pocket): WHICH heads must reproduce and
        WHAT reproducing means are canon (audit_heads = the destructive
        passes off passHeads plus owned keyed, kept to the ruled;
        audit_match = double set-inclusion of the stored cell against
        the rules re-evaluated in rho); this host keeps the counts and
        the try/except robustness as report decoration. Pinned by
        test_verify_canon. A mismatch is a materialization the current
        rules do not reproduce — a tampered .db, or a store saved
        before the rules changed."""
        from .reduce import apply as _apply
        from .kernel import atom as A, from_lam
        from . import defs
        D = self._load(name)
        kinds = {r[0]: r[1] for r in system._pop_rows(D, "derivation")
                 if len(r) >= 2}
        rules = {}
        for r in system._pop_rows(D, "ruleDerives"):
            if len(r) >= 2:
                rules.setdefault(r[1], []).append(r[0])
        classes = system._classify_heads(D)
        audit = set(classes["sweep"]) | set(classes["dred"]) \
            | set(classes["aggwhole"]) \
            | {h for h in classes["keyed"] if kinds.get(h) in system._OWNED}
        checks = []
        for head in sorted(h for h in audit if h in rules):
            recomputed = set()
            for rid in sorted(rules[head]):
                try:
                    with defs.step(D):
                        out = from_lam(_apply(A(rid), D))
                    if isinstance(out, tuple):
                        recomputed |= {tuple(x) for x in out
                                       if isinstance(x, tuple)}
                except Exception:
                    pass                                      # an unevaluable rule
                                                              # reads as a mismatch
            stored = {tuple(r) for r in system._pop_rows(D, head)}
            checks.append({"head": head, "stored": len(stored),
                           "recomputed": len(recomputed),
                           "match": stored == recomputed})
        return {"app": name, "checks": checks}

    def actions(self, name, noun, id):
        """The old engine's actions verb: the legal transitions from the
        entity's current status (HATEOAS: the representation carries its own
        links; the machine read off M, sm_triples the canonical join). Answers
        the machine binding, the current status, and the ⟨event, to⟩ pairs
        available from it."""
        from . import system as _sys
        D = self._load(name)
        smd = _sys.machine_for(D, noun)
        if smd is None:
            return {"app": name, "noun": noun, "id": id, "machine": None,
                    "actions": []}
        # the machine's OBJECT TYPE (the smDef noun) carries the status column;
        # _status_rows is the one reader seam
        gov_noun = next((r[1] for r in _sys._pop_rows(D, "smDef")
                         if len(r) >= 2 and r[0] == smd), smd)
        status = None
        for r in _sys._status_rows(D, gov_noun):
            if len(r) >= 2 and r[0] == id:
                status = r[1]
        triples = _sys.sm_triples(D)
        acts = [{"event": t[1], "to": t[2]} for t in triples
                if status is not None and t[0] == status]
        return {"app": name, "noun": noun, "id": id, "machine": smd,
                "status": status, "actions": acts}

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

    def induce(self, name, ft_id, to_explain=None, bound=None, cap=5000):
        """The abduction primitive (whitepaper §3 + Thm. 4; ported from the
        old engine's induce.rs, its semantics the oracle): enumerate candidate
        bindings for the hidden fact type — the cartesian product of each
        role's domain (declared enum values + the noun's observed population;
        an empty domain collapses the product; an unknown fact type answers
        []) — gate each through the fact type's alethic constraints as a
        BASELINE DELTA (pre-existing violations never reject), gate through
        forward-chain COVERAGE of to_explain when given, score by the app's
        Scoring Rules (each candidate's synthetic hidden rows land in the
        DECLARED Hypothesis_Candidate_has_hidden_<Noun> hook so rules bind
        them; the emitted Confidence Score rows sum — numeric, or 1 per
        categorical row, 0 when none fire), rank descending (enumeration
        order stable on ties), and post-filter by `bound` role pins. Answers
        candidates as data; nothing persists (materialize the convincing one
        via apply)."""
        from . import forml
        from .lam import to_lam, from_lam
        from .reduce import apply as _apx
        from .lam import atom as _A
        D = self._load(name)
        fts = {f[0] for f in system._pop_rows(D, "factType") if f}
        roles = sorted((r[2], r[3]) for r in system._pop_rows(D, "role")
                       if len(r) >= 4 and r[1] == ft_id)
        if ft_id not in fts or not roles:
            return []
        nouns = [n for (_p, n) in roles]

        def domain(noun):
            vals = []
            for vc in system._pop_rows(D, "valueConstraint"):
                if len(vc) >= 2 and vc[0] == noun:
                    vals += re.findall(r"'([^']*)'", str(vc[1]))
            for row in system._pop_rows(D, noun):
                if row and row[0] not in vals:
                    vals.append(row[0])
            for r in system._pop_rows(D, "role"):
                if len(r) >= 4 and r[3] == noun:
                    pos = r[2]
                    for frow in system._pop_rows(D, r[1]):
                        if len(frow) >= pos and frow[pos - 1] != "#" \
                                and frow[pos - 1] not in vals:
                            vals.append(frow[pos - 1])
            return vals

        domains = [domain(n) for n in nouns]
        if any(not d for d in domains):
            return []
        import itertools
        part = system.rmap_partition(D)
        val = forml.validate_for(ft_id, D, part)
        existing = tuple(tuple(r) for r in system.ft_view(D, ft_id, part))

        def violations(rows):
            import pyarest.lam as L
            from . import defs as _dm
            pair = L.SEQ(L.CONS(to_lam(tuple(rows)))(L.CONS(D)(L.NIL)))
            with _dm.step(D):
                out = from_lam(_apx(val, pair))
            return {tuple(v) if isinstance(v, tuple) else (v,)
                    for v in (out[1] if len(out) >= 2 else ())}

        baseline = violations(existing)
        hook = f"Hypothesis_Candidate_has_hidden_{nouns[-1].replace(' ', '_')}"
        hook_declared = hook in fts
        out = []
        idx = -1
        for combo in itertools.islice(itertools.product(*domains), cap):
            idx += 1
            if bound:
                pos_of = {n: p for (p, n) in reversed(roles)}
                if any(pos_of.get(k) and combo[pos_of[k] - 1] != str(v)
                       for k, v in bound.items()):
                    continue
            cand = tuple(combo)
            if violations(existing + (cand,)) - baseline:
                continue                                       # candidate-INTRODUCED only
            hyp_id = f"hyp-{ft_id}-{idx}"
            D2 = _bulk_absorbed_install(D, part, part[ft_id], ft_id, [list(cand)]) \
                if part.get(ft_id, ft_id) != ft_id else \
                _apx(ast.Store(ft_id),
                     _S(to_lam(system._rowsort(set(existing) | {cand})), D))
            if to_explain:
                D3 = system.run_rules(D2, changed=[ft_id])
                # derived rows live in the head's ** cell (the derive cache);
                # a derived ABSORBED head does not land on the column, so the
                # cell — how rules and guards consume derived facts — is the
                # membership read here too
                ok = all(tuple(str(x) for x in e["fact"]) in
                         {tuple(str(x) for x in r)
                          for r in system._pop_rows(D3, e["ft"])}
                         for e in to_explain)
                if not ok:
                    continue
            score = 0
            if hook_declared:
                D4 = _apx(ast.Store("Hypothesis_Candidate"),
                          _S(to_lam(tuple(tuple(r) for r in system._pop_rows(
                              D2, "Hypothesis_Candidate")) + ((hyp_id,),)), D2))
                hrows = tuple(tuple(r) for r in system._pop_rows(D4, hook)) \
                    + ((hyp_id,) + cand[1:],)
                D4 = _apx(ast.Store(hook), _S(to_lam(hrows), D4))
                D4 = system.run_rules(D4, changed=[hook, "Hypothesis_Candidate"])
                for r in system._pop_rows(
                        D4, "Hypothesis_Candidate_has_Confidence_Score"):
                    if len(r) >= 2 and r[0] == hyp_id:
                        try:
                            score += int(str(r[1]))
                        except ValueError:
                            score += 1
            out.append({"id": hyp_id, "confidence_score": score,
                        "hidden": {"ft": ft_id, "fact": list(cand)},
                        "explains": to_explain or []})
        out.sort(key=lambda h: -h["confidence_score"])
        return out

    def ask(self, name, question, plan=None):
        """Read-only Q&A, no LLM in the engine: with a PLAN
        ({fact_type, filter}) the projection query executes, filter values
        compared as strings against the named roles' positions; without one
        the verb answers needs_plan plus the model surface, so ANY caller
        (an MCP sampler, a CLI human) completes the plan and calls again."""
        if plan and plan.get("fact_type"):
            ft = plan["fact_type"]
            rows = self.query(name, ft)
            filt = plan.get("filter") or {}
            if filt:
                D = self._load(name)
                pos_of = {r[3]: r[2] for r in system._pop_rows(D, "role")
                          if len(r) >= 4 and r[1] == ft}
                rows = [r for r in rows
                        if all(k in pos_of and len(r) >= pos_of[k]
                               and str(r[pos_of[k] - 1]) == str(v)
                               for k, v in filt.items())]
            return {"app": name, "question": question, "fact_type": ft,
                    "filter": filt, "rows": rows}
        return {"app": name, "question": question, "needs_plan": True,
                "prompt": ("Translate the question into a plan "
                           "{\"fact_type\": <id>, \"filter\": {<Role Noun>: "
                           "<value>}} against this model, then call ask "
                           "again with it."),
                "model": self.schema(name)}

    def check(self, include_ready=True):
        """Sweep EVERY app: per-app health (ready / stale / library /
        not_found) plus the rolled-up summary. Directory-derived, like the
        registry itself."""
        out = []
        for a in self.list():
            st = self.status(a["name"])
            st["health"] = ("not_found" if not st["exists"] else
                            "library" if st["readings"] == 0 else
                            "stale" if st["stale"] else "ready")
            out.append(st)
        summary = {}
        for st in out:
            summary[st["health"]] = summary.get(st["health"], 0) + 1
        return {"summary": summary,
                "apps": [s for s in out
                         if include_ready or s["health"] != "ready"]}

    def status(self, name):
        """One app's posture without activating it: exists, readings count,
        compiled, and stale (any reading newer than the .db)."""
        import glob as _glob
        d = self._app_dir(name)
        readings = sorted(_glob.glob(os.path.join(d, "readings", "*.md")))
        db = self._db(name)
        compiled = os.path.exists(db)
        newest = max((os.path.getmtime(p) for p in readings), default=0)
        return {"name": name, "exists": os.path.isdir(d),
                "readings": len(readings), "compiled": compiled,
                "stale": (compiled and newest > os.path.getmtime(db))
                         or (not compiled and bool(readings))}

    def create(self, name, text=None):
        """A new app skeleton: <name>/readings/core.md. Refuses on an existing
        app — creation is not mutation."""
        d = self._app_dir(name)
        if os.path.isdir(d):
            raise ValueError(f"app {name!r} already exists")
        os.makedirs(os.path.join(d, "readings"))
        with open(os.path.join(d, "readings", "core.md"), "w",
                  encoding="utf-8") as f:
            f.write(text or f"# {name}\n")
        return {"created": name, "readings": 1}

    def compile_text(self, name, text):
        """The live ADDITIVE compile: the text joins readings/ (the source of
        truth, so a from-scratch rebuild keeps it) and the app recompiles."""
        p = os.path.join(self._app_dir(name), "readings", "_live.md")
        with open(p, "a", encoding="utf-8") as f:
            f.write(text if text.endswith("\n") else text + "\n")
        return self.compile(name)

    def propose(self, name, text):
        """The authoring dry-run: compile the candidate text ATOP the app's
        model on a throwaway store — classification, diagnostics, and the
        would-be declarations — persisting nothing."""
        from . import forml
        D = self._load(name)
        before = {r[0] for r in system._pop_rows(D, "factType") if r}
        D2, rep = forml.compile_model(text, D=D, context_from=D)
        after = {r[0] for r in system._pop_rows(D2, "factType") if r}
        return {"app": name,
                "would_declare": sorted(after - before),
                "unclassified": rep.get("unparsed", []),
                "prose": rep.get("prose", []),
                "diagnostics": rep.get("rule_diagnostics", [])}

    def orient(self):
        cur = self.current()
        return {"active_app": cur, "apps": self.list()}

# ===================== mcp_server =====================
"""The MCP binding (the swap contract, part 2): the old engine's daily-driver
surface served over the Model Context Protocol's stdio transport, newline-delimited
JSON-RPC 2.0, in the stdlib only — a platform binding in the paper's sense (a
server registers its functions; the engine does not change). It carries the orientation and apps family, the read tools (query, sql),
and the write path: apply (eq. create with the receipt) and context (the
last mutation receipt). retract rides the same write path; the tutor/induce family follows.

Run: python -m pyarest.mcp_server <apps_dir>   (or PYAREST_APPS in the env).
"""
import json
import os
import sys

# (intra-protocol import folded)

TOOLS = [
    {"name": "orient",
     "description": "Apps inventory + the active app, one envelope.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "apps_list",
     "description": "Every app under the apps directory (readings/ + .db).",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "apps_current",
     "description": "The active app name, from the persistent marker.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "apps_use",
     "description": "Switch the active app (persists the marker).",
     "inputSchema": {"type": "object", "properties": {
         "name": {"type": "string"}}, "required": ["name"]}},
    {"name": "apps_compile",
     "description": "Compile an app's readings to the lfp and snapshot its .db. "
                    "A from-scratch rebuild by design: supersession is correct.",
     "inputSchema": {"type": "object", "properties": {
         "name": {"type": "string"}}, "required": ["name"]}},
    {"name": "query",
     "description": "A fact type's population from the app's snapshot.",
     "inputSchema": {"type": "object", "properties": {
         "fact_type": {"type": "string"},
         "app": {"type": "string"}}, "required": ["fact_type"]}},
    {"name": "sql",
     "description": "Read-only SQL over the app's snapshot database.",
     "inputSchema": {"type": "object", "properties": {
         "statement": {"type": "string"},
         "app": {"type": "string"}}, "required": ["statement"]}},
    {"name": "apply",
     "description": "Create one fact (eq. create): validate, commit iff no "
                    "alethic violation, log, snapshot. Answers the receipt.",
     "inputSchema": {"type": "object", "properties": {
         "fact_type": {"type": "string"},
         "fact": {"type": "array", "items": {"type": "string"}},
         "app": {"type": "string"}}, "required": ["fact_type", "fact"]}},
    {"name": "retract",
     "description": "Logical deletion, validated: the shrunk population must "
                    "satisfy the schema; commits as a log entry + rebuild.",
     "inputSchema": {"type": "object", "properties": {
         "fact_type": {"type": "string"},
         "fact": {"type": "array", "items": {"type": "string"}},
         "app": {"type": "string"}}, "required": ["fact_type", "fact"]}},
    {"name": "context",
     "description": "The last mutation receipt (committed, violations).",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "get",
     "description": "The 3NF per-entity view: key, absorbed values, unary "
                    "booleans, and the facts the id participates in.",
     "inputSchema": {"type": "object", "properties": {
         "noun": {"type": "string"},
         "id": {"type": "string"},
         "app": {"type": "string"}}, "required": ["noun", "id"]}},
    {"name": "cells",
     "description": "Cell names with row counts (optional substring pattern), "
                    "or one cell's rows via cell=.",
     "inputSchema": {"type": "object", "properties": {
         "pattern": {"type": "string"},
         "cell": {"type": "string"},
         "app": {"type": "string"}}}},
    {"name": "schema",
     "description": "The model surface: object types, fact types with readings "
                    "and roles, constraints.",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string"}}}},
    {"name": "validate",
     "description": "Run the app's constraint validations over the live "
                    "populations; answers the violation report.",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string"}}}},
    {"name": "verify",
     "description": "Re-derive every ruled head and compare against the "
                    "stored populations; answers per-head checks.",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string"}}}},
    {"name": "actions",
     "description": "HATEOAS for one entity: the machine binding, the current "
                    "status (off the RMAP status column), and the legal "
                    "<event, to> transitions from it.",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string"}, "noun": {"type": "string"},
         "id": {"type": "string"}}, "required": ["noun", "id"]}},
    {"name": "synthesize",
     "description": "One entity's full synthesized view by id.",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string"}, "id": {"type": "string"},
         "noun": {"type": "string"}}, "required": ["id"]}},
    {"name": "explain",
     "description": "The derivation trace for an entity or a fact: which "
                    "rules fired, from which premises.",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string"}, "id": {"type": "string"},
         "fact": {"type": "array", "items": {"type": "string"}}},
         "required": ["id"]}},
    {"name": "compile",
     "description": "The live ADDITIVE compile: the text joins the app's "
                    "readings/ (the source of truth, so a rebuild keeps it) "
                    "and the app recompiles.",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string"}, "text": {"type": "string"}},
         "required": ["text"]}},
    {"name": "propose",
     "description": "The authoring dry-run: compile the candidate readings "
                    "ATOP the app's model on a throwaway store — would-be "
                    "declarations, classification, diagnostics — persisting "
                    "nothing.",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string"}, "text": {"type": "string"}},
         "required": ["text"]}},
    {"name": "apps_status",
     "description": "One app's posture without activating it: exists, "
                    "readings count, compiled, stale.",
     "inputSchema": {"type": "object", "properties": {
         "name": {"type": "string"}}, "required": ["name"]}},
    {"name": "apps_create",
     "description": "A new app skeleton: <name>/readings/core.md. Refuses on "
                    "an existing app.",
     "inputSchema": {"type": "object", "properties": {
         "name": {"type": "string"}, "text": {"type": "string"}},
         "required": ["name"]}},
    {"name": "engine_version",
     "description": "The engine and its version.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "apps_check",
     "description": "Sweep EVERY app: per-app health (ready / stale / "
                    "library / not_found) plus the rolled-up summary.",
     "inputSchema": {"type": "object", "properties": {
         "include_ready": {"type": "boolean"}}}},
    {"name": "apps_register",
     "description": "Registration is directory-derived: re-scan the apps "
                    "directory and answer the roster; nothing is written.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "induce",
     "description": "Hypothesis-Candidate search over a fact type — the "
                    "abduction primitive (whitepaper §3, Thm. 4). Enumerate "
                    "bindings for the hidden fact, gate through alethic "
                    "constraints (baseline delta) and forward-chain coverage "
                    "of to_explain, score by the app's Scoring Rules, answer "
                    "ranked candidates. Nothing persists: materialize the "
                    "convincing one via apply.",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string"}, "ft_id": {"type": "string"},
         "to_explain": {"type": "array", "items": {"type": "object"}},
         "bound": {"type": "object"}}, "required": ["ft_id"]}},
    {"name": "ask",
     "description": "Read-only Q&A, no LLM in the engine: pass a plan "
                    "{fact_type, filter} to execute the projection query "
                    "(filter values compared as strings); without one the "
                    "verb answers needs_plan + the model surface for the "
                    "CALLER's sampler to complete.",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string"}, "question": {"type": "string"},
         "plan": {"type": "object"}}, "required": ["question"]}},
    {"name": "tutor_list",
     "description": "List the tutor lessons (tracks easy/medium/hard) with "
                    "titles and goals. The tutor rides a sandbox app "
                    "(_tutor) so lessons never disturb the active app.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "tutor_get",
     "description": "One lesson, parsed: narrative title/goal, the runnable "
                    "fences (each is ONE first-class verb call), and the "
                    "expect predicate the check evaluates.",
     "inputSchema": {"type": "object", "properties": {
         "lesson": {"type": "string",
                    "description": "track/NN, e.g. easy/01"}},
      "required": ["lesson"]}},
    {"name": "tutor_check",
     "description": "Evaluate the lesson's expect predicate against the "
                    "sandbox app; flips passed when the learner's work "
                    "satisfies it.",
     "inputSchema": {"type": "object", "properties": {
         "lesson": {"type": "string"}}, "required": ["lesson"]}},
    {"name": "tutor_reset",
     "description": "Rebootstrap the sandbox: wipe the learner's state, "
                    "copy tutor/domains readings into the _tutor app, "
                    "recompile.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "tutor_apply",
     "description": "apply, scoped to the tutor sandbox app.",
     "inputSchema": {"type": "object", "properties": {
         "fact_type": {"type": "string"},
         "fact": {"type": "array", "items": {}}},
      "required": ["fact_type", "fact"]}},
    {"name": "tutor_query",
     "description": "query, scoped to the tutor sandbox app.",
     "inputSchema": {"type": "object", "properties": {
         "fact_type": {"type": "string"}}, "required": ["fact_type"]}},
    {"name": "tutor_compile",
     "description": "compile readings text into the tutor sandbox app.",
     "inputSchema": {"type": "object", "properties": {
         "text": {"type": "string"}}, "required": ["text"]}},
    {"name": "tutor_propose",
     "description": "propose, scoped to the tutor sandbox app.",
     "inputSchema": {"type": "object", "properties": {
         "text": {"type": "string"}}, "required": ["text"]}},
    {"name": "tutor_actions",
     "description": "actions (HATEOAS transitions), scoped to the tutor "
                    "sandbox app.",
     "inputSchema": {"type": "object", "properties": {
         "noun": {"type": "string"}, "id": {"type": "string"}},
      "required": ["noun", "id"]}},
    {"name": "tutor_authoring",
     "description": "The authoring workflow joined from the sandbox's "
                    "Authoring Step facts: ordered steps with situation, "
                    "guidance, status, and recommended tools; optional "
                    "status filter.",
     "inputSchema": {"type": "object", "properties": {
         "status": {"type": "string"}}}},
    {"name": "select_component",
     "description": "Select a UI Component by intent and constraints from "
                    "the Component registry app (binding doctrine: the "
                    "registry is facts; toolkit implementations register in "
                    "DEFS). Answers ranked {component, role, toolkit, "
                    "symbol, score} records; intent matches the Component "
                    "Role as a case-insensitive substring.",
     "inputSchema": {"type": "object", "properties": {
         "intent": {"type": "string"},
         "traits": {"type": "array", "items": {"type": "string"}},
         "toolkit": {"type": "string"},
         "limit": {"type": "number"},
         "app": {"type": "string"}}, "required": ["intent"]}},
]


# ---- the tutor: lessons over a sandbox app (2026-07-08, ports the legacy
# WASM entry's tutor surface). The sandbox IS an app: _tutor's readings are
# COPIES of tutor/domains (reset == wipe learner state + copy + recompile,
# so the stream/db machinery comes free), and the tutor_* verbs are the
# first-class verbs scoped to it plus the lesson reader and the expect
# checker. The lesson grammar is tutor/lessons/_format.md: runnable fences
# are ONE verb call each; ONE expect predicate per lesson. contains/equals
# objects match by VALUES (the engine's query rows are positional). ----
TUTOR_APP = "_tutor"


def _tutor_root():
    env = os.environ.get("AREST_TUTOR_DIR")
    if env:
        return env
    return os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(
        os.path.abspath(__file__)))), "tutor")


def _tutor_lessons():
    return os.path.join(_tutor_root(), "lessons")


def tutor_reset(reg):
    """Rebootstrap the sandbox: wipe the learner's state (stream + db +
    sidecar), copy tutor/domains into the _tutor app, recompile."""
    import shutil
    src = os.path.join(_tutor_root(), "domains")
    app_dir = os.path.join(reg.root, TUTOR_APP)
    dst = os.path.join(app_dir, "readings")
    os.makedirs(dst, exist_ok=True)
    for fn in os.listdir(dst):
        if fn.endswith(".md"):
            os.remove(os.path.join(dst, fn))
    for fn in sorted(os.listdir(src)):
        if fn.endswith(".md"):
            shutil.copy2(os.path.join(src, fn), os.path.join(dst, fn))
    for fn in list(os.listdir(app_dir)):
        if fn.endswith((".events.jsonl", ".db", ".db.loadcache",
                        ".store.json")):
            try:
                os.remove(os.path.join(app_dir, fn))
            except OSError:
                pass
    rep = reg.compile(TUTOR_APP)
    return {"app": TUTOR_APP, "reset": True, "compiled": rep.get("total")}


# the corpus heads lessons '# Lesson E1: TITLE' (track letter + number);
# _format.md spells '<track>.<num>' — accept both, the corpus wins
_LESSON_HEAD = re.compile(r"^# Lesson\s+([A-Za-z]+)\.?(\d+):\s*(.+)$", re.M)
_LESSON_FENCE = re.compile(r"^~~~\s*(\w+)\s*\n(.*?)^~~~\s*$", re.M | re.S)


def _parse_lesson(text):
    m = _LESSON_HEAD.search(text)
    track, num, title = (m.group(1), m.group(2), m.group(3).strip()) \
        if m else ("", "", "")
    g = re.search(r"^\*\*Goal:\*\*\s*(.+)$", text, re.M)
    fences, expect = [], ""
    for tag, body in _LESSON_FENCE.findall(text):
        if tag == "expect":
            expect = body.strip()
        else:
            fences.append({"tag": tag, "body": body.strip()})
    return {"track": track, "num": num, "title": title,
            "goal": g.group(1).strip() if g else "",
            "fences": fences, "expect": expect}


def _lesson_file(ref):
    track, _, num = ref.partition("/")
    d = os.path.join(_tutor_lessons(), track)
    for fn in sorted(os.listdir(d)):
        if fn.startswith(num + "-") and fn.endswith(".md"):
            return os.path.join(d, fn)
    raise FileNotFoundError(f"no lesson {ref!r}")


def tutor_list():
    root = _tutor_lessons()
    out = []
    for track in sorted(x for x in os.listdir(root)
                        if os.path.isdir(os.path.join(root, x))):
        for fn in sorted(os.listdir(os.path.join(root, track))):
            if not fn.endswith(".md") or fn.startswith("_"):
                continue
            p = _parse_lesson(open(os.path.join(root, track, fn),
                                   encoding="utf-8").read())
            out.append({"lesson": f"{track}/{fn.split('-', 1)[0]}",
                        "track": track, "title": p["title"],
                        "goal": p["goal"]})
    return {"lessons": out}


def tutor_get(ref):
    p = _parse_lesson(open(_lesson_file(ref), encoding="utf-8").read())
    p["lesson"] = ref
    return p


_EXPECT_OPS = {"==": lambda a, b: a == b, ">=": lambda a, b: a >= b,
               "<=": lambda a, b: a <= b, ">": lambda a, b: a > b,
               "<": lambda a, b: a < b}


def _expect_eval(reg, pred):
    """One predicate, _format.md's four forms, against the sandbox app."""
    toks = pred.split()
    if len(toks) >= 4 and toks[0] == "query" and toks[2] == "contains":
        want = json.loads(pred.split("contains", 1)[1].strip())
        rows = reg.query(TUTOR_APP, toks[1])
        vals = {str(v) for v in want.values()}
        return (any(vals <= {str(x) for x in r} for r in rows),
                f"{len(rows)} rows")
    if len(toks) >= 5 and toks[0] in ("query", "list") and toks[2] == "count":
        if toks[0] == "query":
            n = len(reg.query(TUTOR_APP, toks[1]))
        else:
            n = len(reg.get(TUTOR_APP, toks[1], None) or [])
        return (_EXPECT_OPS[toks[3]](n, int(toks[4])), f"count {n}")
    if len(toks) >= 4 and toks[0] == "list" and toks[2] == "contains":
        want = json.loads(pred.split("contains", 1)[1].strip())
        entries = reg.get(TUTOR_APP, toks[1], None) or []
        vals = {str(v) for v in want.values()}
        return (any(vals <= {str(x) for x in
                             (e.values() if isinstance(e, dict) else e)}
                    for e in entries), f"{len(entries)} entries")
    if len(toks) >= 5 and toks[0] == "get" and toks[3] == "equals":
        want = json.loads(pred.split("equals", 1)[1].strip())
        view = reg.get(TUTOR_APP, toks[1], toks[2]) or {}
        flat = json.dumps(view)
        return (all(str(v) in flat for v in want.values()), "view fetched")
    if len(toks) >= 5 and toks[0] == "status" and toks[3] == "is":
        view = reg.get(TUTOR_APP, toks[1], toks[2]) or {}
        return (toks[4] in json.dumps(view), "status read")
    if pred.startswith("violations for apply") and " include " in pred:
        head, cid = pred.rsplit(" include ", 1)
        parts = head.split(None, 5)
        receipt = reg.apply(TUTOR_APP, parts[4], tuple(
            json.loads(head.split(parts[4], 1)[1].strip())))
        hit = any(cid.strip() in str(v) for v in
                  receipt.get("violations", []))
        return (hit, f"committed={receipt.get('committed')}")
    return (False, f"unrecognized expect form: {pred[:60]!r}")


def tutor_check(reg, ref):
    """Evaluate the lesson's ONE expect predicate against the sandbox."""
    pred = tutor_get(ref)["expect"]
    ok, detail = _expect_eval(reg, pred)
    return {"lesson": ref, "expect": pred, "passed": bool(ok),
            "detail": detail}


def select_component(reg, intent, traits=None, toolkit=None, limit=5,
                     app=None):
    """Select a UI Component by intent and constraints (the UI redirect,
    2026-07-08: binding doctrine — the registry is ORDINARY FACTS in a
    registry app, toolkit implementations register in DEFS per the iFactr
    pattern, and selection is this verb). Intent matches case-insensitively
    as a substring against the Component Role (either containment
    direction); wanted traits score +2 present / -1 absent; ranked
    {component, role, toolkit, symbol, score} records answer, highest
    first, ties by component id."""
    name = app or "_components"

    def rows(ft, arity=2):
        try:
            return [tuple(str(x) for x in r) for r in reg.query(name, ft)
                    if isinstance(r, (list, tuple)) and len(r) >= arity]
        except Exception:
            return []
    impls = {}
    # both registry spellings: the corpus's objectified ternary ⟨component,
    # toolkit, symbol⟩ and the flat ⟨component, symbol, toolkit⟩
    for c, tk, sym in rows("Component_is_implemented_by_Toolkit_at_Toolkit_Symbol",
                           arity=3):
        impls.setdefault(c, []).append((sym, tk))
    for c, sym, tk in rows("Component_is_implemented_by_Symbol_in_Toolkit",
                           arity=3):
        impls.setdefault(c, []).append((sym, tk))
    haves = {}
    for c, t in rows("Component_has_Trait"):
        haves.setdefault(c, set()).add(t)
    # binding-level traits key by '<component>.<toolkit>' (the corpus's
    # ImplementationBinding objectification): per-implementation bonuses
    bind_traits = {}
    for b, t in rows("ImplementationBinding_has_Trait"):
        bind_traits.setdefault(b, set()).add(t)
    want = (intent or "").strip().lower()
    wanted = set(traits or [])
    out = []
    for comp, role in rows("Component_has_Component_Role"):
        rl = role.lower()
        if want and want not in rl and rl not in want:
            continue
        got = haves.get(comp, set())
        base = 10 + 2 * len(wanted & got) - len(wanted - got)
        for sym, tk in impls.get(comp, [("", "")]):
            if toolkit and tk != toolkit:
                continue
            bgot = bind_traits.get(f"{comp}.{tk}", set())
            score = base + 2 * len(wanted & bgot)
            out.append({"component": comp, "role": role, "toolkit": tk,
                        "symbol": sym, "score": score})
    out.sort(key=lambda p: (-p["score"], p["component"]))
    return {"app": name, "intent": intent,
            "components": out[:int(limit or 5)]}


def tutor_authoring(reg, status=None):
    """The authoring workflow, joined from the sandbox's Authoring Step
    facts (the legacy readTutorAuthoringWorkflow, positionally: the new
    engine's rows are ⟨step, value⟩ pairs). Steps sort by their order;
    a status filter keeps the steps that use it."""
    def rows(ft):
        try:
            return [tuple(r) for r in reg.query(TUTOR_APP, ft)
                    if isinstance(r, (list, tuple)) and len(r) >= 2]
        except Exception:
            return []
    steps = {}

    def ensure(s):
        return steps.setdefault(s, {"step": s, "order": None,
                                    "situation": None, "guidance": None,
                                    "status": None, "tools": []})
    for s, v in rows("Authoring_Step_has_Authoring_Step_Order"):
        try:
            ensure(str(s))["order"] = int(str(v))
        except ValueError:
            pass
    for s, v in rows("Authoring_Step_applies_in_Authoring_Situation"):
        ensure(str(s))["situation"] = str(v)
    for s, v in rows("Authoring_Step_has_Authoring_Guidance"):
        ensure(str(s))["guidance"] = str(v)
    for s, v in rows("Authoring_Step_uses_Status"):
        ensure(str(s))["status"] = str(v)
    for s, v in rows("Authoring_Step_recommends_Authoring_Tool"):
        ensure(str(s))["tools"].append(str(v))
    for rec in steps.values():
        rec["tools"].sort()
    out = sorted(steps.values(),
                 key=lambda r: (r["order"] is None, r["order"], r["step"]))
    if status is not None:
        out = [r for r in out if r["status"] == status]
    return {"app": TUTOR_APP, "steps": out}


# THE VERB TABLE, first-class from the system (Samuel, 2026-07-04: the
# verbs are not MCP-specific). Every binding — MCP stdio here, the CLI, the
# Rust resident's serve loop, an in-process caller — routes through this one
# table: name -> fn(registry, args). Session verbs need no app; app verbs
# resolve the override-or-active app first. New verbs (synthesize, explain)
# land HERE as Registry-backed entries and every surface gains them at once.
VERSION = "0.9.0"

SESSION_VERBS = {
    "orient": lambda reg, a: reg.orient(),
    "apps_list": lambda reg, a: reg.list(),
    "apps_current": lambda reg, a: {"current": reg.current()},
    "apps_use": lambda reg, a: {"active_app": reg.use(a["name"])},
    "apps_compile": lambda reg, a: reg.compile(a["name"]),
    "apps_status": lambda reg, a: reg.status(a["name"]),
    "apps_create": lambda reg, a: reg.create(a["name"], a.get("text")),
    "apps_check": lambda reg, a: reg.check(
        include_ready=a.get("include_ready", True)),
    "apps_register": lambda reg, a: {
        "registered": [x["name"] for x in reg.list()],
        "note": "directory-derived; nothing written"},
    "engine_version": lambda reg, a: {"engine": "pyarest",
                                      "version": VERSION},
    "context": lambda reg, a: reg.last_receipt
        or {"note": "no mutation this session"},
    # the tutor surface (the legacy WASM entry's port): the lesson reader
    # and checker plus the first-class verbs scoped to the sandbox app
    "tutor_list": lambda reg, a: tutor_list(),
    "tutor_get": lambda reg, a: tutor_get(a["lesson"]),
    "tutor_check": lambda reg, a: tutor_check(reg, a["lesson"]),
    "tutor_reset": lambda reg, a: tutor_reset(reg),
    "tutor_apply": lambda reg, a: APP_VERBS["apply"](reg, TUTOR_APP, a),
    "tutor_query": lambda reg, a: APP_VERBS["query"](reg, TUTOR_APP, a),
    "tutor_compile": lambda reg, a: APP_VERBS["compile"](reg, TUTOR_APP, a),
    "tutor_propose": lambda reg, a: APP_VERBS["propose"](reg, TUTOR_APP, a),
    "tutor_actions": lambda reg, a: APP_VERBS["actions"](reg, TUTOR_APP, a),
    "tutor_authoring": lambda reg, a: tutor_authoring(
        reg, status=a.get("status")),
    "select_component": lambda reg, a: select_component(
        reg, a.get("intent", ""), traits=a.get("traits") or a.get("a11y"),
        toolkit=a.get("toolkit"), limit=a.get("limit", 5),
        app=a.get("app")),
}

APP_VERBS = {
    "query": lambda reg, app, a: {"app": app, "fact_type": a["fact_type"],
                                  "rows": reg.query(app, a["fact_type"])},
    "sql": lambda reg, app, a: {"app": app,
                                "rows": reg.sql(app, a["statement"])},
    "apply": lambda reg, app, a: reg.apply(app, a["fact_type"], a["fact"]),
    "retract": lambda reg, app, a: reg.retract(app, a["fact_type"],
                                               a["fact"]),
    "get": lambda reg, app, a: reg.get(app, a["noun"], a["id"]),
    "cells": lambda reg, app, a: {"app": app,
                                  "cells": reg.cells(
                                      app, pattern=a.get("pattern"),
                                      cell=a.get("cell"))},
    "schema": lambda reg, app, a: reg.schema(app),
    "validate": lambda reg, app, a: reg.validate(app),
    "verify": lambda reg, app, a: reg.verify(app),
    "actions": lambda reg, app, a: reg.actions(app, a["noun"], a["id"]),
    "synthesize": lambda reg, app, a: reg.synthesize(app, a["id"],
                                                     noun=a.get("noun")),
    "explain": lambda reg, app, a: reg.explain(app, a["id"],
                                               fact=a.get("fact")),
    "compile": lambda reg, app, a: reg.compile_text(app, a["text"]),
    "propose": lambda reg, app, a: reg.propose(app, a["text"]),
    "induce": lambda reg, app, a: {"app": app, "candidates": reg.induce(
        app, a["ft_id"], to_explain=a.get("to_explain"),
        bound=a.get("bound"))},
    "ask": lambda reg, app, a: reg.ask(app, a["question"],
                                       plan=a.get("plan")),
}


def verbs():
    """Every verb the system serves, surface-agnostic."""
    return sorted(SESSION_VERBS) + sorted(APP_VERBS)


def _dispatch(reg, name, args):
    if name in SESSION_VERBS:
        return SESSION_VERBS[name](reg, args)
    if name in APP_VERBS:
        app = args.get("app") or reg.current()
        if not app:
            raise ValueError(
                "no app given and no active app set (apps_use first)")
        return APP_VERBS[name](reg, app, args)
    raise ValueError(f"unknown tool {name!r}")


def serve(apps_dir, stdin=None, stdout=None):
    stdin = stdin or sys.stdin
    stdout = stdout or sys.stdout
    reg = apps.Registry(apps_dir, base_dir=apps.default_base())
    for line in stdin:
        line = line.strip()
        if not line:
            continue
        msg = json.loads(line)
        mid = msg.get("id")
        method = msg.get("method")
        if method == "initialize":
            result = {"protocolVersion": msg["params"].get("protocolVersion",
                                                           "2024-11-05"),
                      "capabilities": {"tools": {}},
                      "serverInfo": {"name": "pyarest", "version": VERSION}}
        elif method == "tools/list":
            result = {"tools": TOOLS}
        elif method == "tools/call":
            p = msg.get("params", {})
            try:
                out = _dispatch(reg, p.get("name"), p.get("arguments") or {})
                result = {"content": [{"type": "text",
                                       "text": json.dumps(out, default=str)}]}
            except Exception as e:                             # tool errors are results
                result = {"content": [{"type": "text", "text": str(e)}],
                          "isError": True}
        elif mid is None:
            continue                                           # notification: no reply
        else:
            stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid,
                                     "error": {"code": -32601,
                                               "message": f"unknown {method}"}}) + "\n")
            stdout.flush()
            continue
        if mid is not None:
            stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid,
                                     "result": result}) + "\n")
            stdout.flush()


if __name__ == "__main__":
    serve(sys.argv[1] if len(sys.argv) > 1 else os.environ.get("PYAREST_APPS", "."))
