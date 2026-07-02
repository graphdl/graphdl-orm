"""Phase 3: the self-capture gate (Halpin §13.7 verbatim: the textbook metaschema "is not
rich enough to capture itself"; extend it until it captures any ORM schema, and "to test a
full ORM metaschema, you should be able to populate it with itself").

M is authored as FORML readings and ingested through compile_model like any schema, so
ingesting M produces meta-cells that describe M: the fixpoint. M_MAP bridges each declared
M fact type to the runtime cell that stores its population (M's own rmap); the gate
validates every mapped cell with the constraints M declares for it."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, meta
from pyarest.reduce import apply


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_M_readings_compile_completely():
    _D, rep = meta.M_store()
    assert rep["unparsed"] == []                              # M is inside its own fragment


def test_M_describes_itself():
    D, _ = meta.M_store()
    Dpy = from_lam(D)
    inst = _cell(Dpy, "instanceOf")
    assert ("Object Type", "ObjectType") in inst              # the fixpoint: ObjectType is
    assert ("Fact Type", "ObjectType") in inst                # an instance of ObjectType
    assert ("Role", "ObjectType") in inst
    assert ("OT Kind", "ValueType") in inst
    fts = {f[0] for f in _cell(Dpy, "factType")}
    assert "Object_Type_is_of_OT_Kind" in fts                 # M's fact types are fact rows
    roles = _cell(Dpy, "role")
    assert ("Object_Type_is_of_OT_Kind.1", "Object_Type_is_of_OT_Kind", 1, "Object Type") in roles
    cons = {c[0] for c in _cell(Dpy, "constraint")}
    assert "Object_Type_is_of_OT_Kind_uc" in cons             # and M's constraints too


def test_the_gate_M_validates_its_own_population():
    D, _ = meta.M_store()
    report = meta.self_gate(D)
    assert set(report) == set(meta.M_MAP.values())
    for cell, (v, flag) in report.items():
        assert v == () and flag == "F", (cell, v)             # pristine M passes its own law


def test_the_gate_catches_seeded_corruption():
    D, _ = meta.M_store()
    Dpy = from_lam(D)
    bad = tuple(_cell(Dpy, "instanceOf")) + (("Rogue", "ObjectType"), ("Rogue", "ValueType"))
    D = apply(ast.Store("instanceOf"), L.SEQ(L.CONS(to_lam(bad))(L.CONS(D)(L.NIL))))
    report = meta.self_gate(D)
    v, flag = report["instanceOf"]
    assert flag == "T"                                        # one name, two kinds: caught
    assert ("Rogue", "ObjectType") in v and ("Rogue", "ValueType") in v
