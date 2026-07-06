"""The read-side tools the daily driver uses after orient/query/sql: get (the
3NF per-entity view: key, absorbed values, unaries, and the own-table facts the
id participates in), cells (the store surface: names with counts, or one cell's
rows), and schema (nouns, fact types with readings, constraints). All ride the
same compiled store the write side maintains."""
import pyarest.prims  # noqa: F401
from pyarest import apps


MODEL = """Person(.nr) is an entity type.
Name is a value type.
Person has Name.
Each Person has at most one Name.
Person is vip.
Person likes Person.
Person 'p1' has Name 'Ada'.
Person 'p1' is vip.
Person 'p3' likes Person 'p1'.
"""


def _reg(tmp_path):
    root = str(tmp_path)
    d = tmp_path / "people" / "readings"
    d.mkdir(parents=True)
    (d / "core.md").write_text(MODEL, encoding="utf-8")
    reg = apps.Registry(root)
    reg.compile("people")
    return reg


def test_get_answers_the_per_entity_view(tmp_path):
    reg = _reg(tmp_path)
    v = reg.get("people", "Person", "p1")
    assert v["noun"] == "Person" and v["id"] == "p1"
    assert v["fields"]["Name"] == "Ada"
    assert v["fields"]["is_vip"] is True
    # the m:n participation: p3 likes p1 shows up on p1's view
    assert {"fact_type": "Person_likes_Person", "row": ["p3", "p1"]} in v["facts"]


def test_get_on_an_absent_id_says_so(tmp_path):
    reg = _reg(tmp_path)
    v = reg.get("people", "Person", "nobody")
    assert v["exists"] is False


def test_cells_lists_names_with_counts_and_reads_one(tmp_path):
    reg = _reg(tmp_path)
    listing = reg.cells("people")
    counts = {c["name"]: c["rows"] for c in listing}
    assert counts["Person_has_Name"] == 1
    assert counts["Person_likes_Person"] == 1
    one = reg.cells("people", cell="Person_likes_Person")
    assert one == [["p3", "p1"]]


def test_schema_surfaces_nouns_fact_types_and_constraints(tmp_path):
    reg = _reg(tmp_path)
    s = reg.schema("people")
    nouns = {n["name"]: n for n in s["object_types"]}
    assert nouns["Person"]["kind"] == "ObjectType"
    assert nouns["Name"]["kind"] == "ValueType"
    fts = {f["id"]: f for f in s["fact_types"]}
    assert fts["Person_has_Name"]["roles"] == ["Person", "Name"]
    assert any(c["kind"] == "uniqueness" and c["fact_type"] == "Person_has_Name"
               for c in s["constraints"])
