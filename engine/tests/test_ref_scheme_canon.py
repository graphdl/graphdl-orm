"""#18: the reference-scheme reading handler, canonized. Host _h_ref_scheme delegates
three constant-tag rows (entity->ObjectType, value->ValueType, refScheme) off the two
capture groups; system:h_ref_scheme is the certified-equal lambda twin over ⟨groups,
known, mod⟩ -> ⟨rows, phi⟩. Same pure shape as the h_objectification pilot."""
import pyarest.prims  # noqa: F401
from pyarest import canon, compiler
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

canon.load_all()

SAMPLES = [("Country", "Country Code"), ("A", "B"), ("Task", "Task Nr")]


def _canon(groups):
    r = from_lam(R(A("system:h_ref_scheme"), to_lam((tuple(groups), (), ""))))
    return [(x[0], tuple(x[1])) for x in r[0]], (list(r[1]) if len(r) > 1 else [])


def _host(groups):
    a, o = compiler._h_ref_scheme(groups, None, None)
    return [(c, tuple(row)) for c, row in a], list(o)


def test_ref_scheme_twins_host():
    for g in SAMPLES:
        assert _canon(g) == _host(g), g


def test_ref_scheme_shape():
    a, o = _canon(("Country", "Country Code"))
    assert o == []
    assert a == [("instanceOf", ("Country", "ObjectType")),
                 ("instanceOf", ("Country Code", "ValueType")),
                 ("refScheme", ("Country", "Country Code"))]
