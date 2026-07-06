"""Phase 3 step 4, second half (spec §4.3: "a specific constraint is a fact in M that
selects the expression and binds its Role-sequence arguments"): for the spans-driven
families, validate_for CONSTRUCTS the violation object from the family expression plus
M's spans facts at validate time, instead of resolving a parse-frozen per-instance
object. Proof that M is load-bearing: editing the spans facts changes enforcement."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import ast, defs, forml
from pyarest.reduce import apply


MODEL = """Person is an entity type.
Country is an entity type.
Each Person was born in at most one Country.
"""


def _pd(p, D):
    return L.SEQ(L.CONS(p)(L.CONS(D)(L.NIL)))


def test_uniqueness_is_constructed_from_spans_facts():
    D, _ = forml.compile_model(MODEL)
    # move the recorded span from role 1 to role 2: the SAME schema fact now enforces
    # uniqueness of the Country column, proving the object is built from M, not frozen
    D = apply(ast.Store("spans"), _pd(to_lam((("Person_was_born_in_Country_uc", 2),)), D))
    val = forml.validate_for("Person_was_born_in_Country", D)
    pop = to_lam((("p1", "au"), ("p2", "au")))                # two people, one country
    with defs.step(D):
        _p, v, flag = from_lam(apply(val, _pd(pop, D)))
    assert flag == "T" and set(v) == {("p1", "au"), ("p2", "au")}   # role-2 UC fires


def test_default_spans_still_enforce_role_one():
    D, _ = forml.compile_model(MODEL)
    val = forml.validate_for("Person_was_born_in_Country", D)
    pop = to_lam((("p1", "au"), ("p2", "au")))
    with defs.step(D):
        _p, v, flag = from_lam(apply(val, _pd(pop, D)))
    assert flag == "F" and v == ()                            # distinct people: no violation
