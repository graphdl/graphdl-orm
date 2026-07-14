"""persist — the freeze/thaw slice (SPEC §12-adjacent host machinery).

Transcribed from the quarry's protocol.py persist section (canon-first
0fa14b7c): the symbol-interned sqlite snapshot and content-keyed frozen
ingestion. Adaptations, ledgered: (1) the engine fingerprint hashes THIS
package's sources plus the canon and grammar at the repo root (the rebuild
has no shared/ dir; the canon determines compiled objects, so it must
invalidate snapshots); (2) the sealing-at-rest branches are dormant — no
caller passes seal_key yet; they re-enter with the seal plan's salvage.
The durable event log (JsonlLog, SPEC §12 proper) arrives with the one
gate, not here.
"""
import json
import os
import sqlite3
import zlib

from .lam import to_lam, from_lam


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
    as in D). Symbols by default; compression opt-in (AREST_DB_COMPRESS=zlib)."""
    if compress is None:
        compress = os.environ.get("AREST_DB_COMPRESS", "") or "none"
    if seal_key:
        raise NotImplementedError("sealing at rest re-enters with the seal plan's salvage")
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
                enc = json.dumps(_sym_encode(_conv(c[2]), index, texts),
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
    """The symbolic format only — no legacy reads: a pre-symbols db raises
    sqlite's own no-such-table, and the remedy is recompile."""
    if seal_key:
        raise NotImplementedError("sealing at rest re-enters with the seal plan's salvage")
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
    return to_lam(cells)


# ==================== frozen ingestion: thaw instead of re-ingest =============
def _cache_dir():
    base = os.environ.get("PYAREST_CACHE") or os.environ.get("LOCALAPPDATA")
    if base:
        return os.path.join(base, "arest-rebuild") if "arest-rebuild" not in base else base
    return os.path.join(os.path.expanduser("~"), ".cache", "arest-rebuild")


_ENGINE_FP = []


def _engine_fingerprint():
    """A hash over the sources that determine compiled objects: this host
    package's .py files, the canon, and the grammar at the repo root. A thawed
    D carries COMPILED objects, so any of these changing must invalidate every
    snapshot — text alone would serve stale compiled rules silently."""
    if not _ENGINE_FP:
        import hashlib
        from . import canon as paths
        h = hashlib.sha256()
        pkg = os.path.dirname(os.path.abspath(__file__))
        for fn in sorted(os.listdir(pkg)):
            if fn.endswith(".py"):
                h.update(fn.encode())
                h.update(open(os.path.join(pkg, fn), "rb").read())
        for fn in ("arest.canon", "forml2-grammar.md"):
            p = paths.shared(fn)
            if os.path.exists(p):
                h.update(fn.encode())
                h.update(open(p, "rb").read())
        _ENGINE_FP.append(h.hexdigest()[:16])
    return _ENGINE_FP[0]


def ingest_frozen(text, cache_dir=None, compiler=None):
    """Compile `text` THROUGH the local persistence model: the compiled D
    freezes to a content-keyed sqlite snapshot, and the same text thereafter
    THAWS from disk instead of re-ingesting (definitions are data, so the
    snapshot carries the rules). The key hashes the text AND the engine
    fingerprint: changed text or a changed compiler is a different snapshot,
    so invalidation is by construction. Writes are tmp-then-rename, so racing
    processes cannot tear one."""
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
