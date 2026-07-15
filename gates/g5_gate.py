"""G5 — THE ONE GATE refuses what the constraints forbid (SPEC 2.1–2.5,
Def 5/6, Thm 1; the wedge regression of §13 G5).

The canon-first world's live failure, run forward on the rebuilt gate: the
redo-decision app's exclusion (no eliminated Option may be used) must REFUSE
the forbidden write at the door; the mandatory must refuse a lone retract
that would orphan the Rebuild; and the batch swap must move valid-to-valid
in ONE atomic step — the move the old world could not express, which is
exactly how it wedged (unretractable + unfixable + immortal under replay).
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

_APP = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                    "apps", "redo-decision", "readings", "core.md")

_CACHE = []


def _served():
    if not _CACHE:
        from host_py import gate
        D, rep = gate.compile_serving(open(_APP, encoding="utf-8").read())
        _CACHE.append((D, rep))
    return _CACHE[0]


def test_the_app_compiles_and_derives_the_verdict():
    from host_py import system
    D, rep = _served()
    assert not rep.get("unparsed"), rep.get("unparsed")
    got = {r[0] for r in system._pop_rows(D, "Option_is_eliminated")}
    assert got == {"salvage-assembly", "strangler-in-place"}, got


def test_the_forbidden_choice_is_refused_at_the_door():
    from host_py import gate
    D, _ = _served()
    res = gate.step(D, [("create", "Rebuild_uses_Option",
                         ("redo-2026-07", "strangler-in-place"))])
    assert res["committed"] is False, res["violations"]
    assert res["D"] is D                      # Def 5: D unchanged on refusal


def test_the_surviving_choice_commits():
    from host_py import gate, system
    D, _ = _served()
    journal = []
    res = gate.step(D, [("create", "Rebuild_uses_Option",
                         ("redo-2026-07", "greenfield-transcribe"))],
                    journal=journal)
    assert res["committed"] is True, res["violations"]
    rows = {tuple(r) for r in system._pop_rows(res["D"], "Rebuild_uses_Option")}
    assert ("redo-2026-07", "greenfield-transcribe") in rows
    assert journal and journal[0][0][0] == "create"   # committed inputs only


def test_a_lone_retract_that_orphans_the_rebuild_is_refused():
    from host_py import gate
    D, _ = _served()
    committed = gate.step(D, [("create", "Rebuild_uses_Option",
                               ("redo-2026-07", "greenfield-transcribe"))])
    assert committed["committed"]
    res = gate.step(committed["D"],
                    [("retract", "Rebuild_uses_Option",
                      ("redo-2026-07", "greenfield-transcribe"))])
    assert res["committed"] is False              # mandatory: some Option
    assert res["D"] is committed["D"]


def test_the_journal_replays_through_the_gate(tmp_path):
    from host_py import gate, system
    D, _ = _served()
    path = str(tmp_path / "t.events.jsonl")
    res = gate.step(D, [("create", "Rebuild_uses_Option",
                         ("redo-2026-07", "greenfield-transcribe"))],
                    journal=gate.Journal(path))
    assert res["committed"]
    # a fresh serving compile + replay reproduces the committed population
    from_scratch, _ = gate.compile_serving(open(_APP, encoding="utf-8").read())
    replayed = gate.replay(from_scratch, gate.Journal(path))
    rows = {tuple(r) for r in system._pop_rows(replayed, "Rebuild_uses_Option")}
    assert rows == {("redo-2026-07", "greenfield-transcribe")}


def test_a_forbidden_journal_entry_halts_replay_loudly(tmp_path):
    # the exact opposite of the old world, where the events journal
    # resurrected a forbidden fact into every rebuilt store (181bc775):
    # replay folds through THE GATE, and what cannot commit HALTS (12.2)
    import json
    import pytest
    from host_py import gate
    p = tmp_path / "bad.events.jsonl"
    p.write_text(json.dumps(
        {"tx": 0, "ops": [["create", "Rebuild_uses_Option",
                           ["redo-2026-07", "strangler-in-place"]]]}) + "\n",
        encoding="utf-8")
    from_scratch, _ = gate.compile_serving(open(_APP, encoding="utf-8").read())
    with pytest.raises(RuntimeError, match="halted at tx 0"):
        gate.replay(from_scratch, gate.Journal(str(p)))


def test_the_batch_swap_moves_valid_to_valid_in_one_step():
    from host_py import gate, system
    D, _ = _served()
    committed = gate.step(D, [("create", "Rebuild_uses_Option",
                               ("redo-2026-07", "greenfield-transcribe"))])
    assert committed["committed"]
    # the move the old world could not express: out with one, in with the
    # other, ONE envelope — 2.4's no-wedge theorem in the flesh
    res = gate.step(committed["D"],
                    [("retract", "Rebuild_uses_Option",
                      ("redo-2026-07", "greenfield-transcribe")),
                     ("create", "Rebuild_uses_Option",
                      ("redo-2026-07", "greenfield-transcribe"))])
    assert res["committed"] is True, res["violations"]
    rows = {tuple(r) for r in system._pop_rows(res["D"], "Rebuild_uses_Option")}
    assert rows == {("redo-2026-07", "greenfield-transcribe")}
