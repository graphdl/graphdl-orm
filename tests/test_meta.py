"""The self-capturing metamodel M: describes its own constructs as FFP facts, reflected
back by rho, and validated by its own constraint objects."""
from pyarest import from_lam, to_lam, apply
import pyarest.prims  # noqa: F401
from pyarest import meta as M


def test_object_type_is_an_object_type():
    # self-capture fixpoint: ObjectType occurs among the instances of ObjectType
    ots = from_lam(M.instances_of(M.OBJECTTYPE))
    assert ("ObjectType", "ObjectType") in ots
    assert ("FactType", "ObjectType") in ots               # and the other meta types

def test_M_satisfies_its_own_role_constraint():
    # "each Role is in exactly one FactType" holds of M's own Role population -> no violation
    assert from_lam(M.validate_roles()) == ()

def test_M_can_catch_a_violation_of_its_own_constraint():
    # corrupt M: put one role in two fact types -> the same constraint object flags it
    bad = (("r_dup", "FT_a", 1), ("r_dup", "FT_b", 1), ("r_ok", "FT_c", 1))
    v = from_lam(M.validate_roles(to_lam(bad)))
    assert set(v) == {("r_dup", "FT_a", 1), ("r_dup", "FT_b", 1)}


def test_recompute_frontier_is_bounded_by_the_fact_type():
    # a change to Role_in_FactType re-checks ONLY its constraint and re-fires ONLY the rule
    # that reads it — not the constraint scoped to a different fact type (the streaming bound)
    fr = M.recompute_frontier("Role_in_FactType")
    assert fr["constraints"] == ("uc_role_in_ft",)     # not uc_name (a different scope)
    assert fr["rules"] == ("r_inherit",)
    assert fr["derives"] == ("Inherited_Role",)         # what feeds the next incremental round

def test_frontier_excludes_unrelated_fact_types():
    # a change to ObjectType_has_Name touches its own constraint and no rules
    fr = M.recompute_frontier("ObjectType_has_Name")
    assert fr["constraints"] == ("uc_name",)
    assert fr["rules"] == () and fr["derives"] == ()
