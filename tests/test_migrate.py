"""Migration from an old-engine .db: the cells encoding parses with round-trip
proof (escape alphabet \\ < > , { } = per the old ast.rs escape_atom_for_display;
keyed maps '{k=<<R, v>...>>}' and keyless tuple sequences '<<<R, v>...>>, ...>'),
populations classify against the NEW model (asserted migrates, derived rederives
and VERIFIES, unknown is reported), and the replay lands as batch log entries —
one derive pass, not one validated create per row (the old engine's own atomic
collection apply is the precedent)."""
import json
import os
import sqlite3

import pyarest.prims  # noqa: F401
from pyarest import apps, migrate


def test_parse_cell_keyed_map():
    got = migrate.parse_cell(
        "{k1=<<Task, 179>, <Name, Ada>>, k2=<<Task, 180>, <Name, Bob>>}")
    assert got == [("k1", (("Task", "179"), ("Name", "Ada"))),
                   ("k2", (("Task", "180"), ("Name", "Bob")))]


def test_parse_cell_keyless_sequence():
    got = migrate.parse_cell(
        "<<<Task, 112>, <Task, 113>>, <<Task, 112>, <Task, 114>>>")
    assert got == [(None, (("Task", "112"), ("Task", "113"))),
                   (None, (("Task", "112"), ("Task", "114")))]


def test_parse_cell_unescapes_values():
    # the live corpus: values quoting markup with the old escape alphabet
    got = migrate.parse_cell(
        "{k=<<Task, 429>, <Task Description, URL\\, then \\<a\\> \\= b>>}")
    assert got == [("k", (("Task", "429"),
                          ("Task Description", "URL, then <a> = b")))]


def test_parse_cell_rejects_on_roundtrip_failure():
    assert migrate.parse_cell("{k=<<Task, 1>, garbled") is None
    assert migrate.parse_cell("not cells at all") is None


BASE = """Status is a value type.
Resource is an entity type.
State Machine is an entity type.
State Machine is for Resource.
State Machine is currently in Status.
"""

APP = """Task is an entity type.
Task Subject is a value type.
Task has Task Subject.
Task blocks Task.
Mood is a value type.
The possible values of Mood are 'calm', 'the vibe when the build is red and nobody wants to say it out loud in the standup meeting'.

* Resource is currently in Status iff some State Machine is for that Resource and that State Machine is currently in that Status.
"""

# the base's stored-state idiom: STARRED (derived) with NO rule anywhere — the
# old engine's imperative writers own it (its own comments record the removed
# underspecified rule), so migration must carry it as DATA
BASE_STORED = BASE + """State Machine is for Resource. *
"""


def _mk(tmp_path):
    base_dir = tmp_path / "base"
    base_dir.mkdir()
    (base_dir / "core.md").write_text(BASE_STORED, encoding="utf-8")
    apps_dir = tmp_path / "apps"
    (apps_dir / "board" / "readings").mkdir(parents=True)
    (apps_dir / "board" / "readings" / "app.md").write_text(APP, encoding="utf-8")
    return apps.Registry(str(apps_dir), base_dir=str(base_dir),
                         cache_dir=str(tmp_path / "frozen"))


def _old_db(tmp_path):
    p = str(tmp_path / "old.db")
    con = sqlite3.connect(p)
    con.execute("CREATE TABLE cells (name TEXT PRIMARY KEY, contents TEXT)")
    rows = [
        # asserted, keyed
        ("Task_has_Task_Subject",
         "{a=<<Task, 112>, <Task Subject, Ship the parser>>, "
         "b=<<Task, 113>, <Task Subject, Fix the \\<seam\\>>>}"),
        # asserted, keyless m:n
        ("Task_blocks_Task", "<<<Task, 112>, <Task, 113>>>"),
        # asserted against BASE-declared fact types
        ("State_Machine_is_for_Resource", "{s=<<State Machine, sm1>, <Resource, 112>>}"),
        ("State_Machine_is_currently_in_Status",
         "{s=<<State Machine, sm1>, <Status, in_progress>>}"),
        # derived in the new model: never replayed, only verified
        ("Resource_is_currently_in_Status",
         "{d=<<Resource, 112>, <Status, in_progress>>}"),
        # engine-internal junk: reported, never migrated
        ("SyntheticDerivedCells", "not parseable {{{"),
    ]
    con.executemany("INSERT INTO cells VALUES (?, ?)", rows)
    con.commit()
    con.close()
    return p


def test_the_report_audits_mis_authored_content(tmp_path):
    # old-app readings are known to be mis-authored in places: prose crammed
    # into values, enum members, or ids. The migration FLAGS these (deontic:
    # report and notify, never block) as the swap-time cleanup list.
    reg = _mk(tmp_path)
    p = str(tmp_path / "old2.db")
    con = sqlite3.connect(p)
    con.execute("CREATE TABLE cells (name TEXT PRIMARY KEY, contents TEXT)")
    con.executemany("INSERT INTO cells VALUES (?, ?)", [
        ("Task_has_Task_Subject",
         "{a=<<Task, 112>, <Task Subject, Fix the parser. It broke last week. "
         "We should also consider rewriting the whole module\\, since the "
         "grammar changed underneath it and nobody noticed for a month>>, "
         "b=<<Task, do the thing we discussed at standup yesterday>, "
         "<Task Subject, ok>>}"),
        ("Task_blocks_Task", "<<<Task, 112>, <Task, 113>>>"),
    ])
    con.commit()
    con.close()
    report = migrate.replay_into(reg, "board", p)
    audit = report["authoring"]
    assert any("Task_has_Task_Subject" == a["cell"] and a["kind"] == "prose_value"
               for a in audit)
    assert any(a["kind"] == "prose_id" and "standup" in a["sample"]
               for a in audit)
    # clean rows are not flagged
    assert not any("Task_blocks_Task" == a["cell"] for a in audit)
    # a prose enum member in the READINGS is an authoring defect too
    assert any(a["kind"] == "prose_enum" and "Mood" in a["cell"] for a in audit)


def test_replay_into_migrates_asserted_and_verifies_derived(tmp_path):
    reg = _mk(tmp_path)
    report = migrate.replay_into(reg, "board", _old_db(tmp_path))
    assert report["migrated"]["Task_has_Task_Subject"] == 2
    assert report["migrated"]["Task_blocks_Task"] == 1
    assert reg.query("board", "Task_blocks_Task") == [("112", "113")]
    assert ("113", "Fix the <seam>") in reg.query("board", "Task_has_Task_Subject")
    # STORED STATE (starred, no rule — the engine's imperative writers own it)
    # migrates as data and is never "verified" as a derivation
    assert report["migrated"]["State_Machine_is_for_Resource"] == 1
    assert "State_Machine_is_for_Resource" in report["stored_state"]
    assert "State_Machine_is_for_Resource" not in report["verify"]
    # the derived cell is not replayed; pyarest rederives it (through the
    # migrated stored state) and it MATCHES
    assert "Resource_is_currently_in_Status" not in report["migrated"]
    v = report["verify"]["Resource_is_currently_in_Status"]
    assert v["match"] is True and v["old"] == 1 and v["new"] == 1
    assert "SyntheticDerivedCells" in report["unparsed"]
    # the log carries batch entries, so a fresh compile REPLAYS the migration
    log = [json.loads(x) for x in open(
        os.path.join(str(tmp_path / "apps"), "board", "board.events.jsonl"),
        encoding="utf-8")]
    assert any(e.get("op") == "migrate" for e in log)
    reg.compile("board")                                      # idempotent
    assert reg.query("board", "Task_blocks_Task") == [("112", "113")]
