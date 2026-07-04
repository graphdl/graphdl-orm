"""The DDL generator (the GraphDL day job): readings to relational CREATE TABLE,
Halpin's Rmap output as SQL (book 10.3 for the grouping, 11.12 for the DDL). An
entity's table carries its absorbed functional fact types as columns: the primary
key from the reference scheme, NOT NULL exactly where a mandatory constraint
holds (Halpin: optional roles map to nullable columns), UNIQUE on absorbed
one-to-one columns, a BOOLEAN column per absorbed unary, and REFERENCES where the
column's player is an entity with its own table. An m:n fact type keeps its own
table with the spanning key as PRIMARY KEY and per-role references. The
fix-not-inherit rule rides along: the projection layer must never let one
incomplete entity cascade rows out of the view, so nullable FK columns REFERENCE
without NOT NULL."""
import pyarest.prims  # noqa: F401
from pyarest import ddl, forml


MODEL = """Person(.nr) is an entity type.
Company(.code) is an entity type.
Name is a value type.
Person has Name.
Each Person has at most one Name.
Each Person has some Name.
Person is smoker.
Person works for Company.
Person likes Person.
"""


def test_entity_tables_carry_absorbed_columns():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    out = ddl.generate(D)
    person = out["Person"]
    assert "CREATE TABLE person" in person
    assert "person_nr" in person and "PRIMARY KEY" in person
    assert "name TEXT NOT NULL" in person                     # mandatory + at-most-one
    assert "is_smoker BOOLEAN" in person                      # the unary boolean column


def test_mn_fact_types_get_their_own_keyed_tables():
    D, _ = forml.compile_model(MODEL)
    out = ddl.generate(D)
    works = out["Person_works_for_Company"]
    assert "CREATE TABLE person_works_for_company" in works
    assert "PRIMARY KEY (person_nr, company_code)" in works
    assert "REFERENCES person" in works and "REFERENCES company" in works
    ring = out["Person_likes_Person"]
    assert "PRIMARY KEY (person_nr, person_nr_2)" in ring     # ring: disambiguated


def test_the_script_is_one_executable_document():
    import sqlite3
    D, _ = forml.compile_model(MODEL)
    script = ddl.script(D)
    con = sqlite3.connect(":memory:")
    try:
        con.executescript(script)                             # sqlite accepts it whole
        tables = {r[0] for r in con.execute(
            "SELECT name FROM sqlite_master WHERE type='table'")}
    finally:
        con.close()
    assert {"person", "company", "person_works_for_company",
            "person_likes_person"} <= tables
