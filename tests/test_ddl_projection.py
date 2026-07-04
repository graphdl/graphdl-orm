"""The RMAP projection (the GraphDL day job's second half): populate the generated
tables from the store. Entity rows are the ids playing the entity's roles (the
reference scheme's population, derived — an entity mentioned anywhere exists);
absorbed functional fact types fill columns, absorbed unaries fill booleans, and an
entity missing an absorbed value gets NULL in that column with the row PRESENT —
the old engine's dangling-FK cascade (one incomplete entity emptying ten nouns from
the view) is impossible by construction. Own-table fact types insert row per fact."""
import sqlite3

import pyarest.prims  # noqa: F401
from pyarest import ddl, forml, system


MODEL = """Person(.nr) is an entity type.
Name is a value type.
Person has Name.
Each Person has at most one Name.
Person is vip.
Person likes Person.
Person 'p1' has Name 'Ada'.
Person 'p2' has Name 'Grace'.
Person 'p3' likes Person 'p1'.
Person 'p1' is vip.
"""


def _project(model):
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    D = system.run_rules(D)
    con = sqlite3.connect(":memory:")
    ddl.project(D, con)
    return con


def test_entity_rows_absorbed_columns_and_booleans():
    con = _project(MODEL)
    rows = dict(con.execute(
        "SELECT person_nr, name FROM person ORDER BY person_nr").fetchall())
    # p3 exists (it plays the liker role) with a NULL name: no cascade, ever
    assert rows == {"p1": "Ada", "p2": "Grace", "p3": None}
    vip = dict(con.execute("SELECT person_nr, is_vip FROM person").fetchall())
    assert vip["p1"] == 1 and vip["p2"] == 0 and vip["p3"] == 0


def test_own_table_fact_types_project_row_per_fact():
    con = _project(MODEL)
    likes = con.execute("SELECT * FROM person_likes_person").fetchall()
    assert likes == [("p3", "p1")]


def test_the_projection_is_queryable_the_graphdl_way():
    con = _project(MODEL)
    got = con.execute(
        "SELECT p.name FROM person p JOIN person_likes_person l "
        "ON l.person_nr_2 = p.person_nr").fetchall()
    assert got == [("Ada",)]                                  # p3 likes p1, named Ada


def test_reserved_word_identifiers_are_quoted():
    # the base metamodel projects a 'constraint' table (the old .db carries it);
    # every generated identifier must be quoted or sqlite refuses the schema
    model = """Constraint is an entity type.
Name is a value type.
Constraint has Name.
Each Constraint has at most one Name.
Constraint 'c1' has Name 'uc1'.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    D = system.run_rules(D)
    con = sqlite3.connect(":memory:")
    counts = ddl.project(D, con)
    assert counts.get("Constraint") == 1
    rows = con.execute('SELECT constraint_id, name FROM "constraint"').fetchall()
    assert rows == [("c1", "uc1")]


def test_a_roleless_fact_type_is_skipped_not_malformed_sql():
    # an ALL-LOWERCASE degenerate reading records a fact type with no role rows
    # (Title-case words would mine as implicit nouns and carry roles); the
    # projection must skip it with a named None count, never emit
    # 'CREATE TABLE x (, PRIMARY KEY ())'
    D, rep = forml.compile_model(MODEL + "\nwidgets frob gadgets.\n")
    # the selfhost default REPORTS the degenerate (no fields, no
    # classification, decline tracking) instead of recording a roleless fact
    # type: strictly safer, same guarantee below (no malformed SQL, ever)
    flagged = rep["unparsed"] + rep.get("prose", [])
    assert any("widgets frob gadgets" in s for s in flagged)
    D = system.run_rules(D)
    con = sqlite3.connect(":memory:")
    counts = ddl.project(D, con)
    assert counts.get("widgets_frob_gadgets", "absent") in (None, "absent")
    tables = {r[0] for r in con.execute(
        "SELECT name FROM sqlite_master WHERE type='table'")}
    assert "widgets_frob_gadgets" not in tables
    assert "person" in tables                                 # the rest still projects


def test_reprojection_adds_columns_for_new_fact_types():
    # schema evolution on a LIVE db (the gms-1993 filing exposed it): a later
    # compile declares a new absorbed fact type, and re-projecting into the
    # same connection must ALTER the new column in — CREATE IF NOT EXISTS
    # alone never revisits an existing table
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    D = system.run_rules(D)
    con = sqlite3.connect(":memory:")
    ddl.project(D, con)
    D2, rep2 = forml.compile_model(MODEL + """Person is an algorithm author.
Rank is a value type.
Person holds Rank.
Each Person holds at most one Rank.
Person 'p2' is an algorithm author.
Person 'p2' holds Rank 'fellow'.
""")
    assert rep2["unparsed"] == []
    D2 = system.run_rules(D2)
    ddl.project(D2, con)
    rows = dict(con.execute(
        "SELECT person_nr, is_an_algorithm_author FROM person").fetchall())
    assert rows == {"p1": 0, "p2": 1, "p3": 0}
    ranks = dict(con.execute(
        "SELECT person_nr, rank FROM person").fetchall())
    assert ranks["p2"] == "fellow" and ranks["p1"] is None
