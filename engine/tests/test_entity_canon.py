"""#18: the entity-type reading handler, canonized (the first CONDITIONAL class). Stage-1
now delivers two groups g0=name, g1=refmode ('' when absent); host _h_entity rides refMode
only when g1 is present. system:h_entity is the certified-equal twin over ⟨groups, known,
mod⟩ -> ⟨rows, phi⟩ with COND(eq(g1,''), single-row, two-rows)."""
import pyarest.prims  # noqa: F401
from pyarest import canon, compiler
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

canon.load_all()

# (name, refmode) pairs — '' is Stage-1's absent-refmode, the polyglot form the canon tests
SAMPLES = [("User", "Email"), ("Country", "Country Code"), ("Object Type", ""), ("A", "")]


def _canon(groups):
    r = from_lam(R(A("system:h_entity"), to_lam((tuple(groups), (), ""))))
    return [(x[0], tuple(x[1])) for x in r[0]], (list(r[1]) if len(r) > 1 else [])


def _host(groups):
    a, o = compiler._h_entity(groups, None, None)
    return [(c, tuple(row)) for c, row in a], list(o)


def test_entity_twins_host():
    for g in SAMPLES:
        assert _canon(g) == _host(g), g


def test_entity_shape():
    assert _canon(("User", "Email")) == (
        [("instanceOf", ("User", "ObjectType")), ("refMode", ("User", "Email"))], [])
    assert _canon(("Thing", "")) == ([("instanceOf", ("Thing", "ObjectType"))], [])
