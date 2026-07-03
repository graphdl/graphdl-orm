"""Persistence per the platform arc: a driver is a registered def set (the paper names
upsert as the binding); the EVENT LOG is the primary durable form (each committed step
as ⟨tx, fact_type, fact⟩ — the τ log made durable, so bitemporality and durability
unify), snapshots are replay optimizations. jsonl carries the log; sqlite carries cells
one-to-one with per-step transactions; replay is re-ingestion through the same create.
Fact cells must survive the roundtrip exactly; definitions recompile from readings."""
import os
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, persist, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


MODEL = """Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Customer places Order.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
"""


def _fact_cells(D):
    out = {}
    for c in from_lam(D):
        if not (isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"):
            continue
        rows = c[2]
        if isinstance(rows, tuple) and all(
                isinstance(r, tuple) and all(not isinstance(x, tuple) for x in r)
                for r in rows):
            out.setdefault(c[1], set()).update(rows)
    return out


def _flow(D):
    D = apply(ast.Store("Order_status"), S(to_lam((("o1", "In Cart"),)), D))
    return apply(A(2), system.create(D, "Customer_places_Order", to_lam(("c1", "o1"))))


def test_sqlite_roundtrips_the_fact_cells(tmp_path):
    D, _ = forml.compile_model(MODEL)
    D = _flow(D)
    path = os.path.join(str(tmp_path), "store.db")
    persist.save_sqlite(D, path)
    D2 = persist.load_sqlite(path)
    assert _fact_cells(D2) == _fact_cells(D)                  # cells one-to-one


def test_the_jsonl_log_replays_to_the_same_state(tmp_path):
    # the event log is the primary form: appending each committed step and replaying
    # through the SAME create rebuilds the state (facts are the source of truth)
    path = os.path.join(str(tmp_path), "steps.jsonl")
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.Store("Order_status"), S(to_lam((("o1", "In Cart"),)), D))
    log = persist.JsonlLog(path)
    D = log.create(D, "Customer_places_Order", ("c1", "o1"))
    D = log.create(D, "Customer_places_Order", ("c2", "o1"))  # re-fire: idempotent facts
    fresh, _ = forml.compile_model(MODEL)
    fresh = apply(ast.Store("Order_status"), S(to_lam((("o1", "In Cart"),)), fresh))
    replayed = persist.replay(fresh, path)
    assert _fact_cells(replayed) == _fact_cells(D)
    assert _fact_cells(replayed)["Order_status"] == {("o1", "Placed")}


def test_the_durable_log_carries_transaction_time(tmp_path):
    path = os.path.join(str(tmp_path), "steps.jsonl")
    D, _ = forml.compile_model(MODEL)
    log = persist.JsonlLog(path)
    D = log.create(D, "Customer_places_Order", ("c1", "o1"))
    entries = persist.read_log(path)
    assert entries[0]["tx"] == 1 and entries[0]["ft"] == "Customer_places_Order"
    assert tuple(entries[0]["fact"]) == ("c1", "o1")


def test_a_refused_step_is_not_logged(tmp_path):
    path = os.path.join(str(tmp_path), "steps.jsonl")
    RING = MODEL + "Order blocks Order.\nOrder blocks Order is irreflexive.\n"
    D, _ = forml.compile_model(RING)
    log = persist.JsonlLog(path)
    vo = forml.validate_for("Order_blocks_Order", D)
    D = log.create(D, "Order_blocks_Order", ("o1", "o1"), validate_obj=vo)
    assert persist.read_log(path) == []                       # ERROR commits nothing
