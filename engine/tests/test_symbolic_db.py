"""The symbolic db format (2026-07-08): symbols by default — every
distinct leaf atom stored once, typed via its own JSON — compression
opt-in (AREST_DB_COMPRESS=zlib), and NO legacy reads: a pre-symbols db
fails loudly and the remedy is recompile."""
import sqlite3

import pyarest.prims  # noqa: F401
from pyarest import forml
from pyarest.protocol import save_sqlite, load_sqlite
from pyarest.lam import from_lam


MODEL = """Status is a value type.
Ticket is an entity type.
Ticket has Status.
Ticket 't1' has Status 'open'.
Ticket 't2' has Status 'open'.
"""


def _cells(D):
    return tuple(c for c in from_lam(D)
                 if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL")


def test_round_trip_preserves_every_cell(tmp_path):
    D, _ = forml.compile_model(MODEL)
    p = str(tmp_path / "t.db")
    save_sqlite(D, p)
    assert _cells(load_sqlite(p)) == _cells(D)


def test_symbols_stored_once_and_typed(tmp_path):
    D, _ = forml.compile_model(MODEL)
    p = str(tmp_path / "t.db")
    save_sqlite(D, p)
    con = sqlite3.connect(p)
    texts = [t for (t,) in con.execute("SELECT text FROM symbols")]
    fmt = dict(con.execute("SELECT key, value FROM format"))
    con.close()
    assert texts.count('"open"') == 1          # the repeated atom, once
    assert fmt["encoding"] == "symbolic-v1"
    assert fmt["compress"] == "none"
    # types survive: ints stay ints, strings stay strings
    D2 = load_sqlite(p)
    flat = []

    def walk(v):
        if isinstance(v, tuple):
            for x in v:
                walk(x)
        else:
            flat.append(v)
    walk(_cells(D2))
    assert any(isinstance(v, int) and not isinstance(v, bool) for v in flat)
    assert any(isinstance(v, str) for v in flat)


def test_compression_is_opt_in_and_round_trips(tmp_path):
    D, _ = forml.compile_model(MODEL)
    p = str(tmp_path / "t.db")
    save_sqlite(D, p, compress="zlib")
    con = sqlite3.connect(p)
    fmt = dict(con.execute("SELECT key, value FROM format"))
    blob = con.execute("SELECT contents FROM cells LIMIT 1").fetchone()[0]
    con.close()
    assert fmt["compress"] == "zlib"
    assert isinstance(blob, bytes)             # opaque on purpose
    assert _cells(load_sqlite(p)) == _cells(D)


def test_no_legacy_reads(tmp_path):
    # a plain-format db (the pre-symbols era) fails loudly
    import pytest
    p = str(tmp_path / "old.db")
    con = sqlite3.connect(p)
    con.execute("CREATE TABLE cells (ord INTEGER PRIMARY KEY, "
                "name TEXT, contents TEXT)")
    con.execute("INSERT INTO cells VALUES (0, '\"x\"', '[[\"a\"]]')")
    con.commit()
    con.close()
    with pytest.raises(sqlite3.OperationalError):
        load_sqlite(p)
