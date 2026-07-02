"""The metamodel layer reads everything off the store (Phase 3 step: no module-constant
mirrors): instance extents by θ₁ selection over the instanceOf cell, and the
recomputation frontier off the constraint/ruleReads/ruleDerives cells that ingestion
itself wrote when it parsed the schema."""
import pyarest.prims  # noqa: F401
from pyarest import forml, meta


MODEL = """Person is an entity type.
Car is an entity type.
Name is a value type.
Person has Name.
Each Person has at most one Name.
*Each FastCarDriver is some Person who drives some Car that is fast.
"""


def test_instances_of_reads_the_store():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    assert {"Person", "Car"} <= meta.instances_of(D, "ObjectType")
    assert "Name" in meta.instances_of(D, "ValueType")


def test_frontier_constraints_read_off_the_constraint_cell():
    D, _ = forml.compile_model(MODEL)
    assert "Person_has_Name_uc" in meta.affected_constraints(D, "Person_has_Name")
    assert meta.affected_constraints(D, "Person_drives_Car") == ()


def test_frontier_rules_read_off_the_rule_cells():
    D, _ = forml.compile_model(MODEL)
    # the parsed role path reads Person-drives-Car and Car-is-fast, and derives the subtype
    assert meta.affected_rules(D, "Person_drives_Car") == ("FastCarDriver_rule",)
    assert meta.affected_rules(D, "Car_is_fast") == ("FastCarDriver_rule",)
    assert meta.affected_rules(D, "Person_has_Name") == ()
    fr = meta.recompute_frontier(D, "Person_drives_Car")
    assert fr["rules"] == ("FastCarDriver_rule",)
    assert fr["derives"] == ("FastCarDriver",)                # feeds the next incremental round
