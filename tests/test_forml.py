"""compile ∘ parse = create over M (D3 / Cor. closure): a schema change is an ordinary
command on M's population, validated by M's own constraints. No compiler subsystem."""
import pytest
from pyarest import from_lam, apply
from pyarest.lam import atom as A
import pyarest.prims  # noqa: F401
from pyarest import forml, meta
from pyarest import constraints as C

_Dstate = lambda result: apply(A(2), result)                 # D' from ⟨o, D'⟩ as an FFP object


def cell_of(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL" and c[1] == name:
            return c[2]
    return ()


def test_compile_is_create_over_M():
    (o, Dp) = from_lam(forml.compile("Person is an entity type", meta.initial_D()))
    (m2, v) = o
    assert ("Person", "ObjectType") in m2
    assert ("ObjectType", "ObjectType") in m2         # self-capture preserved through compile
    assert ("Person", "ObjectType") in cell_of(Dp, "instanceOf")   # committed into M's population

def test_compile_validated_by_M_own_constraint():
    # "each name has one kind" = uniqueness on the name (role 1) of instanceOf. Declaring Person
    # as BOTH an entity type and a value type binds one name to two kinds -> M's own constraint
    # rejects the schema change, on the same create/validate path as any command (Cor. closure).
    uc = C.uniqueness([1])
    r1 = forml.compile("Person is an entity type", meta.initial_D(), constraints=[uc])
    D1 = _Dstate(r1)                                   # thread D' forward as the FFP state
    assert ("Person", "ObjectType") in cell_of(from_lam(D1), "instanceOf")  # first declaration commits
    (o2, D2) = from_lam(forml.compile("Person is a value type", D1, constraints=[uc]))
    (_m2, v) = o2
    assert set(v) != set()                            # Person bound to two kinds -> violation
    assert ("Person", "ValueType") not in cell_of(D2, "instanceOf")  # not committed; M unchanged

def test_value_type_declaration():
    (o, _Dp) = from_lam(forml.compile("Name is a value type", meta.initial_D()))
    assert ("Name", "ValueType") in o[0]

def test_reading_outside_R_is_rejected():
    with pytest.raises(ValueError):
        forml.parse("Person who drives Car")


def test_nf_is_idempotent():
    # Prop. Spec: nf = verbalize ∘ parse is idempotent on the fragment R
    for r in ("Person is an entity type", "Name is a value type"):
        assert forml.nf(r) == r
        assert forml.nf(forml.nf(r)) == forml.nf(r)
