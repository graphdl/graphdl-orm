"""Storage is a swappable 3NF DRIVER (Samuel, 2026-07-05: swappable between
sqlite, R2, postgresql, clickhouse, anything usable as a 3NF storage driver).
The interface mirrors arest's StorageBackend, the whole-store form over cells:
save commits the cell store, load rehydrates it, and SQL backends additionally
serve the 3NF relational projection and query. The sqlite driver is one
implementation; the backend is not the design."""
import os

import pytest

import pyarest.prims  # noqa: F401
from pyarest import persist, system


MODEL = ("Status is a value type.\nTicket is an entity type.\n"
         "Ticket has Status.\n")


def _mk(tmp_path, sub="apps"):
    base = tmp_path / sub
    (base / "flow" / "readings").mkdir(parents=True)
    (base / "flow" / "readings" / "app.md").write_text(MODEL, encoding="utf-8")
    from pyarest import apps
    return apps.Registry(str(base), cache_dir=str(tmp_path / "fz"))


def test_the_default_storage_is_sqlite(tmp_path):
    reg = _mk(tmp_path)
    reg.compile("flow")
    assert os.path.exists(
        os.path.join(str(tmp_path / "apps" / "flow"), "flow.db"))


def test_swapping_storage_redirects_save_and_load(tmp_path):
    # point the registry at the memory driver: the cell store lives in memory,
    # NO .db is written, and a reload rehydrates the applied fact
    persist.MemoryStorage.clear()
    reg = _mk(tmp_path)
    reg.storage = "memory"
    reg.compile("flow")
    receipt = reg.apply("flow", "Ticket_has_Status", ("t1", "open"))
    assert receipt["committed"]

    assert not os.path.exists(
        os.path.join(str(tmp_path / "apps" / "flow"), "flow.db"))
    D = reg._load("flow")
    rows = {tuple(r) for r in system._pop_rows(D, "Ticket_has_Status")}
    assert ("t1", "open") in rows                            # round-trips memory


def test_the_sql_surface_is_a_backend_capability(tmp_path):
    # a SQL backend (sqlite) serves the 3NF query surface; an object backend
    # (memory) has none and says so, rather than crashing
    reg = _mk(tmp_path)
    reg.compile("flow")
    rows = reg.sql("flow", "SELECT COUNT(*) FROM sqlite_master")
    assert rows[0][0] >= 1

    persist.MemoryStorage.clear()
    reg2 = _mk(tmp_path, sub="apps2")
    reg2.storage = "memory"
    reg2.compile("flow")
    with pytest.raises(ValueError):
        reg2.sql("flow", "SELECT 1")


def test_the_driver_interface_mirrors_arest(tmp_path):
    # the whole-store form: save commits, load rehydrates, exists reports
    drv = persist.resolve_storage_driver("memory", str(tmp_path), "app")
    assert isinstance(drv, persist.StorageDriver)
    assert drv.sql is False
    assert drv.load() is None and drv.exists() is False
