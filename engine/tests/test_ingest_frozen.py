"""The local persistence model applied to ingestion: a compiled model FREEZES to a
content-keyed sqlite snapshot and THAWS on the next process instead of re-ingesting.
Sound because definitions are DATA (compiled FFP objects are nested atoms/sequences,
the same shape as fact rows), so save_sqlite/load_sqlite round-trips the WHOLE D,
rules included — the load-bearing test runs run_rules on a thawed D. The key is the
model text's hash: changed text is a different snapshot, so invalidation is by
construction; writes go tmp-then-rename so racing processes cannot tear a snapshot."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import ast, forml, system, persist
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


MODEL = """Person(.id) is an entity type.
Person is a parent of Person.
Person is an ancestor of Person.
Person1 is an ancestor of Person2 if Person1 is a parent of Person2.
"""


def test_sqlite_roundtrips_definitions_a_thawed_D_still_derives(tmp_path):
    D, _ = forml.compile_model(MODEL)
    p = str(tmp_path / "m.sqlite")
    persist.save_sqlite(D, p)
    D2 = persist.load_sqlite(p)
    D2 = apply(ast.Store("Person_is_a_parent_of_Person"),
               S(to_lam((("a", "b"),)), D2))
    D2 = system.run_rules(D2)                                 # rules THAWED from disk fire
    assert _cell(from_lam(D2), "Person_is_an_ancestor_of_Person") == {("a", "b")}


def test_ingest_frozen_freezes_once_and_thaws_after(tmp_path):
    cache = str(tmp_path / "cache")
    calls = []
    real = forml.compile_model

    def probe(text, D=None):
        calls.append(1)
        return real(text, D)

    forml.compile_model = probe
    try:
        D1 = persist.ingest_frozen(MODEL, cache_dir=cache)    # cold: ingests + freezes
        assert calls == [1]
        D2 = persist.ingest_frozen(MODEL, cache_dir=cache)    # warm: thaws, no ingest
        assert calls == [1]
    finally:
        forml.compile_model = real
    assert {c[1] for c in from_lam(D2) if isinstance(c, tuple) and c and c[0] == "CELL"} \
        == {c[1] for c in from_lam(D1) if isinstance(c, tuple) and c and c[0] == "CELL"}


def test_changed_text_is_a_different_snapshot(tmp_path):
    cache = str(tmp_path / "cache")
    persist.ingest_frozen(MODEL, cache_dir=cache)
    persist.ingest_frozen(MODEL + "Person is mortal.\n", cache_dir=cache)
    import os
    assert len(os.listdir(cache)) == 2                        # content-keyed: no aliasing


def test_a_changed_engine_invalidates_the_snapshot(tmp_path):
    # a thawed D carries COMPILED objects, so the key covers the compiler too:
    # after an engine edit yesterday's snapshot must not serve stale rules
    cache = str(tmp_path / "cache")
    persist.ingest_frozen(MODEL, cache_dir=cache)
    real = list(persist._ENGINE_FP)
    persist._ENGINE_FP[:] = ["different-engine"]
    try:
        persist.ingest_frozen(MODEL, cache_dir=cache)
    finally:
        persist._ENGINE_FP[:] = real
    import os
    assert len(os.listdir(cache)) == 2
