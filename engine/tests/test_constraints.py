"""A constraint implemented in FFP and reflected back as (rho c):P = V_c."""
from pyarest import apply, to_lam, from_lam
from pyarest.lam import atom as A
import pyarest.prims  # noqa: F401
from pyarest import constraints as C


def ev(op, data):
    return from_lam(apply(op, to_lam(data)))


def test_uniqueness_finds_duplicate_keyed_tuples():
    uc = C.uniqueness([1])                                   # role 1 must be unique
    dup = (("a", "x"), ("b", "y"), ("a", "z"))              # a appears twice
    assert set(ev(uc, dup)) == {("a", "x"), ("a", "z")}     # both a-tuples violate

def test_uniqueness_satisfied_is_empty():
    uc = C.uniqueness([1])
    ok = (("a", "x"), ("b", "y"), ("c", "z"))
    assert ev(uc, ok) == ()                                  # V_c = phi -> no violation

def test_composite_uniqueness():
    uc = C.uniqueness([1, 2])                                # roles (1,2) jointly unique
    pop = (("a", "1", "p"), ("a", "2", "q"), ("a", "1", "r"))  # (a,1) twice
    assert set(ev(uc, pop)) == {("a", "1", "p"), ("a", "1", "r")}

def test_ring_irreflexive():
    assert set(ev(C.ring_irreflexive(), (("a", "b"), ("c", "c"), ("d", "d")))) == {("c", "c"), ("d", "d")}

def test_ring_symmetric():
    # ⟨a,b⟩ has no reverse -> violation; ⟨x,y⟩ with ⟨y,x⟩ present -> ok
    assert set(ev(C.ring_symmetric(), (("a", "b"), ("x", "y"), ("y", "x")))) == {("a", "b")}

def test_value_range():
    assert set(ev(C.value_range(2, 1, 10), (("a", 5), ("b", 0), ("c", 11), ("d", 10)))) == {("b", 0), ("c", 11)}

def test_mandatory_and_subset():
    m = ev(C.mandatory(), ((("a",), ("b",), ("c",)), (("a",), ("c",))))   # b plays nothing
    assert set(m) == {("b",)}
    s = ev(C.subset(), ((("x",), ("y",)), (("x",),)))                     # y not in the superset
    assert set(s) == {("y",)}


def test_constraint_reflects_through_rho():
    # define(name, c) makes rho(name) denote (rho c): the ORM meta-object IS the reflected FFP
    uc = C.uniqueness([1])
    C.register_constraint("uc_person", uc)
    pop = (("a", "x"), ("a", "z"), ("b", "y"))
    direct = from_lam(apply(uc, to_lam(pop)))
    reflected = from_lam(apply(A("uc_person"), to_lam(pop)))   # via the name, through rho
    assert direct == reflected
    assert set(reflected) == {("a", "x"), ("a", "z")}
