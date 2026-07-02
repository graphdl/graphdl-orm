"""Role-path -> F_S: a derivation rule is a conjunctive query (join-project over fact types,
per ORM->Datalog); recursion is resolved by derive = lfp F_S. Compiled rules reduce by the one mu."""
from pyarest import from_lam, to_lam, apply
import pyarest.prims  # noqa: F401
from pyarest import system


def ev(op, data):
    return from_lam(apply(op, to_lam(data)))


def test_two_fact_type_join_derivation():
    # FastCarDriver(x) <- drives(x, y), isFast(y)  — join on the Car, project the Person
    rule = system.join_rule2(2, [1])
    drives = (("alice", "car1"), ("bob", "car2"))
    is_fast = (("car1",),)
    assert set(ev(rule, (drives, is_fast))) == {("alice",)}   # alice drives a fast car; bob doesn't


def test_one_step_join():
    # grandparent(x,z) <- parent(x,y), parent(y,z)  — one application of the self-join rule
    rule = system.join_rule(2, [1, 3])
    parent = (("a", "b"), ("b", "c"))
    assert set(ev(rule, parent)) == {("a", "c")}


def test_recursive_transitive_closure():
    # ancestor <- link ; link(x,y), ancestor(y,z)  — derive_of takes the least fixed point
    derive = system.derive_of([system.join_rule(2, [1, 3])])
    links = (("a", "b"), ("b", "c"), ("c", "d"))
    assert set(ev(derive, links)) == {("a", "b"), ("b", "c"), ("c", "d"),
                                       ("a", "c"), ("b", "d"), ("a", "d")}   # full transitive closure
