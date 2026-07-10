"""#18: the metadata reading handlers (data_type, ref_mode), canonized. Host _h_meta is a
factory emitting one row (cell, (g0,)) into the named cell; system:h_meta_data_type and
system:h_meta_ref_mode are the certified-equal lambda twins over ⟨groups, known, mod⟩ ->
⟨⟨⟨cell, ⟨g0⟩⟩⟩, phi⟩. Exercises the single-element construction S2(CONS, x)."""
import pyarest.prims  # noqa: F401
from pyarest import canon, compiler
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

canon.load_all()

CASES = [
    ("system:h_meta_data_type", compiler._h_meta("data_type")),
    ("system:h_meta_ref_mode", compiler._h_meta("ref_mode")),
]
SAMPLES = [("VIN",), ("Email Address",), ("x",)]


def _canon(name, groups):
    r = from_lam(R(A(name), to_lam((tuple(groups), (), ""))))
    return [(x[0], tuple(x[1])) for x in r[0]], (list(r[1]) if len(r) > 1 else [])


def _host(fn, groups):
    a, o = fn(groups, None, None)
    return [(c, tuple(row)) for c, row in a], list(o)


def test_meta_handlers_twin_host():
    for name, fn in CASES:
        for g in SAMPLES:
            assert _canon(name, g) == _host(fn, g), (name, g)


def test_meta_shape():
    a, o = _canon("system:h_meta_data_type", ("VIN",))
    assert o == []
    assert a == [("data_type", ("VIN",))]
