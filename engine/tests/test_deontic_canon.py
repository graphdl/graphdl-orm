"""The deontic-value family, moved from host-only Filter compositions into
constraints.canon (2026-07-09 canon-completeness audit). A row is treated as a
SET; forbidden flags rows where (row setminus values) != row (the row carries a
forbidden value); obligatory_value flags the complement (a row lacking every
obligated value). Gated by the ABSOLUTE result. Both reduce identically in the
Rust host (differential-covered primitives: theta:Filter/setminus, not, eq,
CONST, id)."""
import pyarest.prims  # noqa: F401
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

# population: <x,a> carries no forbidden value; <y,forbidden> and <z,forbidden> do
P = to_lam((("x", "a"), ("y", "forbidden"), ("z", "forbidden")))
VALUES = to_lam(("forbidden",))


def flag(name):
    built = R(A(name), VALUES)
    return sorted(from_lam(R(built, P)))


def test_deontic_forbidden():
    # rows carrying a forbidden value
    assert flag("constraints:deontic_forbidden") == [("y", "forbidden"), ("z", "forbidden")]


def test_deontic_obligatory_value():
    # rows LACKING every obligated value (the complement)
    assert flag("constraints:deontic_obligatory_value") == [("x", "a")]
