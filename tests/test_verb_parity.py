"""Two more first-class verbs from the old engine's surface, table-backed:
validate (the app's constraint validation over the SETTLED store — eq. create
only ever judged candidates, but instance facts in readings ingest unvalidated
and deontic violations commit by design, so a compiled store can carry drift)
and verify (the migration report's derived-population parity check turned
in-place self-audit: re-evaluate each fully-derived head's rules over the
settled store and compare with the stored cell). Both are Registry methods
behind one-line entries in the verb table, so every binding gains them at
once — the MCP layer is untouched beyond the table."""
import json
import sqlite3

import pyarest.prims  # noqa: F401
from pyarest import apps, mcp_server


UC_APP = """Person(.id) is an entity type.
Name is a value type.
Person has Name.
Each Person has at most one Name.
Person 'p1' has Name 'Ada'.
Person 'p1' has Name 'Grace'.
"""

DERIVED_APP = """Status is a value type.
Resource is an entity type.
State Machine is an entity type.
State Machine is for Resource.
State Machine is currently in Status.

* Resource is currently in Status iff some State Machine is for that Resource and that State Machine is currently in that Status.
"""


def _mk(tmp_path, name, readings):
    d = tmp_path / name / "readings"
    d.mkdir(parents=True)
    (d / "app.md").write_text(readings, encoding="utf-8")
    reg = apps.Registry(str(tmp_path))
    reg.compile(name)
    return reg


def test_validate_reports_the_settled_stores_violations(tmp_path):
    # the readings assert two Names for p1 under 'at most one': compile
    # ingests them unjudged, and validate is the audit that catches it
    reg = _mk(tmp_path, "v1", UC_APP)
    out = reg.validate("v1")
    assert out["app"] == "v1"
    assert len(out["violations"]) == 1
    v = out["violations"][0]
    assert v["fact_type"] == "Person_has_Name"
    assert "uniqueness" in v["kinds"]
    assert v["alethic"] is True
    assert sorted(map(tuple, v["offenders"])) == [("p1", "Ada"),
                                                  ("p1", "Grace")]


def test_validate_answers_clean_after_the_offender_retracts(tmp_path):
    reg = _mk(tmp_path, "v2", UC_APP)
    receipt = reg.retract("v2", "Person_has_Name", ["p1", "Grace"])
    assert receipt["committed"] is True
    assert reg.validate("v2") == {"app": "v2", "violations": []}


def test_verify_matches_the_settled_derivations(tmp_path):
    reg = _mk(tmp_path, "v3", DERIVED_APP)
    reg.apply("v3", "State_Machine_is_for_Resource", ["sm1", "r1"])
    reg.apply("v3", "State_Machine_is_currently_in_Status", ["sm1", "active"])
    out = reg.verify("v3")
    assert out["app"] == "v3"
    assert out["checks"] == [{"head": "Resource_is_currently_in_Status",
                              "stored": 1, "recomputed": 1, "match": True}]


def test_verify_catches_a_tampered_materialization(tmp_path):
    # append a row to the stored derived cell BEHIND the engine's back: the
    # rules do not reproduce it, and verify says so
    reg = _mk(tmp_path, "v4", DERIVED_APP)
    reg.apply("v4", "State_Machine_is_for_Resource", ["sm1", "r1"])
    reg.apply("v4", "State_Machine_is_currently_in_Status", ["sm1", "active"])
    head = "Resource_is_currently_in_Status"
    con = sqlite3.connect(reg._db("v4"))
    (contents,) = con.execute("SELECT contents FROM cells WHERE name=?",
                              (json.dumps(head),)).fetchone()
    rows = json.loads(contents)
    rows.append(["r9", "zombie"])
    con.execute("UPDATE cells SET contents=? WHERE name=?",
                (json.dumps(rows), json.dumps(head)))
    con.commit()
    con.close()
    (check,) = reg.verify("v4")["checks"]
    assert check == {"head": head, "stored": 2, "recomputed": 1,
                     "match": False}


def test_the_verbs_ride_the_first_class_table(tmp_path):
    # one table, every surface: _dispatch resolves the active app for
    # validate and honors the app override for verify
    reg = _mk(tmp_path, "v5", UC_APP)
    reg.use("v5")
    assert "validate" in mcp_server.verbs()
    assert "verify" in mcp_server.verbs()
    out = mcp_server._dispatch(reg, "validate", {})
    assert out["app"] == "v5" and out["violations"]
    assert mcp_server._dispatch(reg, "verify", {"app": "v5"}) == \
        {"app": "v5", "checks": []}
