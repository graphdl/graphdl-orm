"""Subtyping fixed at the semantics, not the display: subtype instances ARE supertype
instances, so the subtype declaration itself installs the upward-inclusion derivation
rule (super(x) ← sub(x), the ordinary rule machinery: ruleAtom facts, ~d variants,
semi-naive through chains), and rule CLAUSES resolve up to supertype-declared fact
types (the lift both engines' tests demanded). The subset constraint remains the check;
the rule is the meaning."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


CHAIN = """Party is an entity type.
Person is an entity type.
Priest is an entity type.
Person is a subtype of Party.
Priest is a subtype of Person.
"""


def test_membership_propagates_up_the_chain():
    D, rep = forml.compile_model(CHAIN)
    assert rep["unparsed"] == []
    D = apply(ast.Store("Priest"), S(to_lam((("fr-brown",),)), D))
    D = system.run_rules(D)
    Dpy = from_lam(D)
    assert ("fr-brown",) in _cell(Dpy, "Person")              # a Priest IS a Person
    assert ("fr-brown",) in _cell(Dpy, "Party")               # and IS a Party


def test_inclusion_rides_the_ordinary_rule_machinery():
    D, _ = forml.compile_model(CHAIN)
    Dpy = from_lam(D)
    rules = _cell(Dpy, "ruleDerives")
    assert any(head == "Party" for (_rid, head) in rules)     # installed BY the declaration
    atoms = _cell(Dpy, "ruleAtom")
    assert any(ft == "Person" for (_rid, _pos, ft) in atoms)  # semi-naive variants exist


def test_a_supertype_keyed_fact_type_reaches_subtype_instances():
    MODEL = CHAIN + """Address is a value type.
Party resides at Address.
Party1 is housed if Party1 resides at Address2.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    D = apply(ast.Store("Priest"), S(to_lam((("fr-brown",),)), D))
    D = apply(ast.Store("Party_resides_at_Address"), S(to_lam((("fr-brown", "st-mary"),)), D))
    D = system.run_rules(D)
    Dpy = from_lam(D)
    assert ("fr-brown",) in _cell(Dpy, "Party_is_housed")     # the Party-keyed rule fires
    assert ("fr-brown",) in _cell(Dpy, "Party")               # via his propagated membership
