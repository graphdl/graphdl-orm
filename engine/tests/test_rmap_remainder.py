"""The RMAP remainder. (1) The inverse UC anchors to the FACT TYPE as a real role-2
uniqueness (spans position computed), so a doubly-functional fact type is detectable.
(2) 1:1 grouping favors fewer nulls (Halpin §10.3): a 1:1 fact type absorbs into the
side whose player is MANDATORY (every instance plays, so its column has no holes),
defaulting to role 1. (3) Index maintenance rides the ONE step: a refused routed write
leaves the index unchanged and a committed one records the key, atomically with the row.
(4) Step 5's constraint mapping: a value constraint on an absorbed column refuses the
routed write that violates it."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


ONE_TO_ONE = """Person is an entity type.
Passport is an entity type.
Person holds Passport.
Each Person holds at most one Passport.
For each Passport, exactly one Person holds that Passport.
"""


def test_the_inverse_uc_is_a_role2_uniqueness_on_the_fact_type():
    D, rep = forml.compile_model(ONE_TO_ONE)
    assert rep["unparsed"] == []
    Dpy = from_lam(D)
    cons = _cell(Dpy, "constraint")
    assert any(f[1] == "uniqueness" and f[2] == "Person_holds_Passport" and "_inv_uc" in f[0]
               for f in cons if len(f) >= 3)
    spans = _cell(Dpy, "spans")
    assert any("_inv_uc" in s[0] and s[1] == 2 for s in spans)


def test_one_to_one_absorbs_into_the_fewer_nulls_side():
    D, _ = forml.compile_model(ONE_TO_ONE)
    part = system.rmap_partition(D)
    # every Passport is held (mandatory), so the Passport side has no holes: absorb there
    assert part["Person_holds_Passport"] == "Passport"


ABSORBED = """Order(.OrderId) is an entity type.
Rating is a value type.
Order has Rating.
Each Order has at most one Rating.
The possible values of Rating are 1, 2, 3, 4, 5.
"""


def test_a_mapped_value_constraint_refuses_the_routed_write():
    D, rep = forml.compile_model(ABSORBED)
    assert rep["unparsed"] == []
    part = system.rmap_partition(D)
    assert part["Order_has_Rating"] == "Order"
    Dp = apply(A(2), system.create(D, "Order_has_Rating", to_lam(("o1", 9))))
    assert system.ft_view(Dp, "Order_has_Rating", part) == set()     # 9 refused
    Dp2 = apply(A(2), system.create(D, "Order_has_Rating", to_lam(("o1", 3))))
    assert system.ft_view(Dp2, "Order_has_Rating", part) == {("o1", 3)}


def test_index_maintenance_rides_the_one_step():
    D, _ = forml.compile_model(ABSORBED)
    part = system.rmap_partition(D)
    Dp = apply(A(2), system.create(D, "Order_has_Rating", to_lam(("o1", 9))))
    assert _cell(from_lam(Dp), "Order") == set()              # refused: no index entry
    Dp2 = apply(A(2), system.create(D, "Order_has_Rating", to_lam(("o1", 3))))
    assert ("o1",) in _cell(from_lam(Dp2), "Order")           # committed: indexed
    Dp3 = apply(A(2), system.create(Dp2, "Order_has_Rating", to_lam(("o1", 3))))
    assert len([r for r in from_lam(apply(ast.FetchPop("Order"), Dp3)) if r == ("o1",)]) == 1
