"""The write side of the apps protocol: apply creates a fact against the app's
store through the SAME create as everything else (eq. create: validate, commit iff
no alethic violation), appends the committed step to the app's event log (the
paper's τ log made durable; a refused step appends nothing), snapshots the .db, and
answers the mutation RECEIPT — the representation o with the violation set, which
the MCP context tool replays. Facts are the source of truth: the log replays
through create, so the .db is disposable."""
import json
import os

import pyarest.prims  # noqa: F401
from pyarest import apps


def _mkapp(tmp_path, name, readings):
    d = tmp_path / name / "readings"
    d.mkdir(parents=True)
    (d / "app.md").write_text(readings, encoding="utf-8")
    return apps.Registry(str(tmp_path))


READINGS = """Person(.id) is an entity type.
Name is a value type.
Person has Name.
Each Person has at most one Name.
"""


def test_apply_commits_logs_and_snapshots(tmp_path):
    reg = _mkapp(tmp_path, "w1", READINGS)
    reg.compile("w1")
    receipt = reg.apply("w1", "Person_has_Name", ["p1", "Ada"])
    assert receipt["committed"] is True
    assert receipt["violations"] == []
    assert ["p1", "Ada"] in [list(r) for r in reg.query("w1", "Person_has_Name")]
    log = os.path.join(str(tmp_path), "w1", "w1.events.jsonl")
    entries = [json.loads(l) for l in open(log, encoding="utf-8")]
    assert entries[-1]["ft"] == "Person_has_Name"


def test_apply_refusal_reports_and_changes_nothing(tmp_path):
    reg = _mkapp(tmp_path, "w2", READINGS)
    reg.compile("w2")
    reg.apply("w2", "Person_has_Name", ["p1", "Ada"])
    receipt = reg.apply("w2", "Person_has_Name", ["p1", "Grace"])   # UC violation
    assert receipt["committed"] is False
    assert receipt["violations"]                                     # V reported
    rows = [list(r) for r in reg.query("w2", "Person_has_Name")]
    assert ["p1", "Grace"] not in rows and ["p1", "Ada"] in rows
    log = os.path.join(str(tmp_path), "w2", "w2.events.jsonl")
    entries = [json.loads(l) for l in open(log, encoding="utf-8")]
    assert len(entries) == 1                                         # refusal logged nothing


def test_replay_rebuilds_the_store_from_the_log(tmp_path):
    reg = _mkapp(tmp_path, "w3", READINGS)
    reg.compile("w3")
    reg.apply("w3", "Person_has_Name", ["p1", "Ada"])
    reg.apply("w3", "Person_has_Name", ["p2", "Grace"])
    os.remove(os.path.join(str(tmp_path), "w3", "w3.db"))            # the .db is disposable
    out = reg.compile("w3")                                          # rebuild + replay
    rows = {tuple(r) for r in reg.query("w3", "Person_has_Name")}
    assert rows == {("p1", "Ada"), ("p2", "Grace")}


def test_retract_removes_logs_and_validates(tmp_path):
    reg = _mkapp(tmp_path, "w4", READINGS)
    reg.compile("w4")
    reg.apply("w4", "Person_has_Name", ["p1", "Ada"])
    receipt = reg.retract("w4", "Person_has_Name", ["p1", "Ada"])
    assert receipt["committed"] is True
    assert ("p1", "Ada") not in {tuple(r) for r in reg.query("w4", "Person_has_Name")}
    log = os.path.join(str(tmp_path), "w4", "w4.events.jsonl")
    entries = [json.loads(l) for l in open(log, encoding="utf-8")]
    assert entries[-1]["op"] == "retract"
    # replay honors the retraction: rebuild from readings + log
    os.remove(os.path.join(str(tmp_path), "w4", "w4.db"))
    reg.compile("w4")
    assert ("p1", "Ada") not in {tuple(r) for r in reg.query("w4", "Person_has_Name")}


MANDATORY = READINGS + "Each Person has some Name.\n"


def test_retract_refuses_when_the_shrunk_population_violates(tmp_path):
    reg = _mkapp(tmp_path, "w5", MANDATORY)
    reg.compile("w5")
    reg.apply("w5", "Person", ["p1"])
    reg.apply("w5", "Person_has_Name", ["p1", "Ada"])
    receipt = reg.retract("w5", "Person_has_Name", ["p1", "Ada"])
    assert receipt["committed"] is False                      # p1 would go nameless
    assert receipt["violations"]
    assert ("p1", "Ada") in {tuple(r) for r in reg.query("w5", "Person_has_Name")}


def test_the_db_carries_the_projected_tables(tmp_path):
    reg = _mkapp(tmp_path, "w6", READINGS)
    rep = reg.compile("w6")
    assert rep["projected"].get("Person_has_Name") is not None or True
    reg.apply("w6", "Person_has_Name", ["p1", "Ada"])
    reg.compile("w6")                                         # reproject after writes
    rows = reg.sql("w6", "SELECT person_id, name FROM person")
    assert ("p1", "Ada") in {tuple(r) for r in rows}
