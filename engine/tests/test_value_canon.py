"""#18: the value-type reading handler, canonized — system:h_entity's conditional twin
bar the kind tag. Stage-1 delivers g0=name, g1=refmode ('' when absent); host _h_value
rides refMode only when g1 is present; system:h_value is the certified-equal lambda twin."""
import pyarest.prims  # noqa: F401
from pyarest import canon, compiler
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

canon.load_all()

SAMPLES = [("Email", ""), ("Task Nr", ""), ("Code", "Code Value"), ("B", "")]


def _canon(groups):
    r = from_lam(R(A("system:h_value"), to_lam((tuple(groups), (), ""))))
    return [(x[0], tuple(x[1])) for x in r[0]], (list(r[1]) if len(r) > 1 else [])


def _host(groups):
    a, o = compiler._h_value(groups, None, None)
    return [(c, tuple(row)) for c, row in a], list(o)


def test_value_twins_host():
    for g in SAMPLES:
        assert _canon(g) == _host(g), g


def test_value_shape():
    assert _canon(("Email", "")) == ([("instanceOf", ("Email", "ValueType"))], [])
    assert _canon(("Code", "Code Value")) == (
        [("instanceOf", ("Code", "ValueType")), ("refMode", ("Code", "Code Value"))], [])
