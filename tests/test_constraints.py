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

def test_constraint_reflects_through_rho():
    # define(name, c) makes rho(name) denote (rho c): the ORM meta-object IS the reflected FFP
    uc = C.uniqueness([1])
    C.register_constraint("uc_person", uc)
    pop = (("a", "x"), ("a", "z"), ("b", "y"))
    direct = from_lam(apply(uc, to_lam(pop)))
    reflected = from_lam(apply(A("uc_person"), to_lam(pop)))   # via the name, through rho
    assert direct == reflected
    assert set(reflected) == {("a", "x"), ("a", "z")}
