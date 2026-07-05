"""The event stream is an INTERFACE swapped through registration, the same
DEFS-override discipline as the rule engines and the storage layer (Samuel,
2026-07-05: the jsonl file was an undesigned implementation choice, not the
design). A committed step is APPENDED to the sink and the stream is READ back
for reconstruction; the file is one implementation, swappable for a memory
buffer, a broadcast Durable Object (the arest tier), or any backend."""
import os

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


def test_the_default_sink_is_the_file(tmp_path):
    reg = _mk(tmp_path)
    reg.compile("flow")
    reg.apply("flow", "Ticket_has_Status", ("t1", "open"))
    log = os.path.join(str(tmp_path / "apps" / "flow"), "flow.events.jsonl")
    assert os.path.exists(log)                         # behavior preserved


def test_swapping_the_sink_redirects_the_whole_stream(tmp_path):
    # point the registry at the memory sink: the committed event lands in
    # memory, NO file is written, and a recompile reconstructs from it
    persist.MemoryEventSink.clear()
    reg = _mk(tmp_path)
    reg.event_sink = "memory"
    reg.compile("flow")
    receipt = reg.apply("flow", "Ticket_has_Status", ("t1", "open"))
    assert receipt["committed"]

    captured = persist.MemoryEventSink._store.get("flow", [])
    assert any(e.get("fact") == ["t1", "open"] for e in captured)
    log = os.path.join(str(tmp_path / "apps" / "flow"), "flow.events.jsonl")
    assert not os.path.exists(log)                     # the file was bypassed

    # a recompile replays from the memory sink, so the fact survives
    reg.compile("flow")
    D = reg._load("flow")
    rows = {tuple(r) for r in system._pop_rows(D, "Ticket_has_Status")}
    assert ("t1", "open") in rows


def test_the_sink_interface_is_two_operations(tmp_path):
    # the interface is exactly append and read, the Connector's two names
    persist.MemoryEventSink.clear()
    sink = persist.resolve_event_sink("memory", str(tmp_path), "app")
    assert isinstance(sink, persist.EventSink)
    sink.append({"ft": "F", "fact": ["a", "b"]})
    sink.append({"ft": "F", "fact": ["c", "d"]})
    assert sink.read() == [{"ft": "F", "fact": ["a", "b"]},
                           {"ft": "F", "fact": ["c", "d"]}]
