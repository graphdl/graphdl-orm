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
from pyarest import ddl, forml, system


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
    # identifiers are quoted: the base metamodel projects reserved-word tables
    # (constraint, transition), so every emitted name wears double quotes
    assert 'CREATE TABLE "person"' in person
    assert '"person_nr"' in person and "PRIMARY KEY" in person
    assert '"name" TEXT NOT NULL' in person                   # mandatory + at-most-one
    assert '"is_smoker" BOOLEAN' in person                    # the unary boolean column


def test_mn_fact_types_get_their_own_keyed_tables():
    D, _ = forml.compile_model(MODEL)
    out = ddl.generate(D)
    works = out["Person_works_for_Company"]
    assert 'CREATE TABLE "person_works_for_company"' in works
    assert 'PRIMARY KEY ("person_nr", "company_code")' in works
    assert 'REFERENCES "person"' in works and 'REFERENCES "company"' in works
    ring = out["Person_likes_Person"]
    assert 'PRIMARY KEY ("person_nr", "person_nr_2")' in ring  # ring: disambiguated


def test_mandatory_hardens_the_mandated_players_column():
    # 'Each Status is defined in some SMD' mandates STATUS; the fact type
    # absorbs into Status's table, so its smd column hardens — NOT NULL rides
    # the absorbed column exactly when the mandated player IS the absorbing
    # table (Halpin 11.12; the tasks migration tripped the inverted form)
    model = """Status(.name) is an entity type.
SMD(.name) is an entity type.
Status is defined in SMD.
Each Status is defined in at most one SMD.
Each Status is defined in some SMD.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    status = ddl.generate(D)["Status"]
    # absorbed INTO status (functional role on Status): NOT NULL is correct here
    assert '"smd_name" TEXT NOT NULL' in status


def test_subject_extraction_prefers_the_longest_type_name():
    # 'State Machine' and 'State Machine Definition' both declared: the subject
    # of a constraint over the longer name must never truncate to the shorter
    # (set iteration order made it nondeterministic — the tasks base tripped it)
    model = """Status(.name) is an entity type.
State Machine(.id) is an entity type.
State Machine Definition(.name) is an entity type.
State Machine Definition has initial Status.
Each State Machine Definition has some initial Status.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    rows = [tuple(c) for c in system._pop_rows(D, "constraint")
            if len(c) >= 4 and c[1] == "mandatory"]
    assert rows and rows[0][3] == "State Machine Definition"


def test_the_projection_is_soft_where_generate_is_hard():
    # the old engine's projected tables are a DATA MIRROR (its
    # state_machine_definition carries no NOT NULL beyond the key), so migrated
    # populations with a missing mandatory value land as NULL rows instead of
    # crashing the compile; generate() keeps the honest DDL
    import sqlite3
    model = """Person(.nr) is an entity type.
Name is a value type.
Person has Name.
Each Person has at most one Name.
Each Person has some Name.
Person likes Person.
Person 'p1' has Name 'Ada'.
Person 'p2' likes Person 'p1'.
"""
    D, rep = forml.compile_model(model)
    assert '"name" TEXT NOT NULL' in ddl.generate(D)["Person"]
    from pyarest import system as _sys
    D = _sys.run_rules(D)
    con = sqlite3.connect(":memory:")
    counts = ddl.project(D, con)                              # p2 has no Name
    assert counts["Person"] == 2
    rows = dict(con.execute("SELECT person_nr, name FROM person").fetchall())
    assert rows == {"p1": "Ada", "p2": None}


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
