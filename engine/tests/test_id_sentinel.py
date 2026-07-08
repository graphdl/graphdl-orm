"""The id-sentinel guard (the phi phantom's write-side fix, filed
2026-07-08): an apply whose key position carries the phi atom or the
empty string refuses BEFORE evaluation — those are leaks, never
modeling intent. The log replay stays ungated (history is history)
and retract stays open (cleaning needs to reach such rows)."""
import pyarest.prims  # noqa: F401
from pyarest.protocol import Registry


class _NoLoad(Registry):
    # apply refuses at the guard before _load runs, so a registry with
    # no apps directory proves the guard fires first
    def _load(self, name):
        raise AssertionError("the guard must refuse before any load")


def test_phi_and_empty_ids_refuse_before_evaluation():
    reg = _NoLoad.__new__(_NoLoad)
    for bad in ("φ", ""):
        r = reg.apply("anyapp", "Task_is_started", (bad,))
        assert r["committed"] is False
        assert r["violations"][0][0] == "id-sentinel"


def test_a_normal_id_reaches_the_engine():
    reg = _NoLoad.__new__(_NoLoad)
    try:
        reg.apply("anyapp", "Task_is_started", ("t1",))
        raise SystemExit("expected the load assertion")
    except AssertionError as e:
        assert "guard must refuse before any load" in str(e)
