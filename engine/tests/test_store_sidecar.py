"""The resident's food: every Registry snapshot writes <name>.store.json beside
the .db, one serve-protocol line persisted (set_store's payload: d, process,
overrides, cases). The Rust resident boots an app by feeding the file through
the same ingestion path a --serve line takes, so the sidecar and the .db must
stay in lockstep through compile AND apply."""
import json
import os

import pyarest.prims  # noqa: F401


MODEL = """Status is a value type.
Ticket is an entity type.
Ticket has Status.
"""


def _mk(tmp_path):
    base = tmp_path / "apps"
    (base / "flow" / "readings").mkdir(parents=True)
    (base / "flow" / "readings" / "app.md").write_text(MODEL, encoding="utf-8")
    from pyarest import apps as _apps
    return _apps.Registry(str(base), cache_dir=str(tmp_path / "fz"))


def _encoded_store(reg):
    from pyarest import persist
    from pyarest.lam import from_lam
    from pyarest.polyglot import _conv
    return _conv(from_lam(persist.load_sqlite(reg._db("flow"))))


def test_compile_writes_the_sidecar_in_lockstep_with_the_db(tmp_path):
    reg = _mk(tmp_path)
    reg.compile("flow")
    side = os.path.join(os.path.dirname(reg._db("flow")), "flow.store.json")
    with open(side, encoding="utf-8") as f:
        payload = json.load(f)
    assert set(payload) == {"d", "process", "overrides", "cases"}
    assert payload["overrides"] == 1 and payload["cases"] == []
    assert payload["d"] == _encoded_store(reg)


def test_apply_refreshes_the_sidecar(tmp_path):
    reg = _mk(tmp_path)
    reg.compile("flow")
    side = os.path.join(os.path.dirname(reg._db("flow")), "flow.store.json")
    with open(side, encoding="utf-8") as f:
        before = json.load(f)["d"]
    receipt = reg.apply("flow", "Ticket_status", ("t1", "open"))
    assert receipt["committed"]
    with open(side, encoding="utf-8") as f:
        after = json.load(f)["d"]
    assert after != before
    assert after == _encoded_store(reg)
