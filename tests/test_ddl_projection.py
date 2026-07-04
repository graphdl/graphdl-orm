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
