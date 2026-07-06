"""Constraint families read off Halpin ch. 7 and §10.3 (verbatim definitions):

Ring (§7.3): asymmetric  iff xRy → ¬yRx (implies irreflexive);
             antisymmetric iff x≠y & xRy → ¬yRx;
             intransitive  iff xRy & yRz → ¬xRz;
             acyclic       iff no path via the relation from an object back to itself.
Frequency (§7.2): each member of pop(roles) occurs there exactly/at-least/at-most n times;
             local to the role population, not the object type.
External UC (§10.3): "equivalent to an internal uniqueness constraint spanning [the two
             columns] in the natural join of the two tables".
Subtyping (§10.3 step 0): absorb subtypes into their top supertype; subtype membership
             entails supertype membership.
"""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, system
from pyarest import constraints as C
from pyarest.reduce import apply


def ev(obj, data):
    return set(from_lam(apply(obj, to_lam(data))))


def test_ring_asymmetric():
    v = ev(C.ring_asymmetric(), (("a", "b"), ("b", "a"), ("c", "d"), ("e", "e")))
    assert v == {("a", "b"), ("b", "a"), ("e", "e")}          # both directions + reflexive
    assert ev(C.ring_asymmetric(), (("a", "b"), ("b", "c"))) == set()


def test_ring_antisymmetric():
    v = ev(C.ring_antisymmetric(), (("a", "b"), ("b", "a"), ("e", "e")))
    assert v == {("a", "b"), ("b", "a")}                      # reflexive pairs are ALLOWED


def test_ring_intransitive():
    v = ev(C.ring_intransitive(), (("a", "b"), ("b", "c"), ("a", "c"), ("x", "y")))
    assert v == {("a", "c")}                                  # the forbidden composition
    assert ev(C.ring_intransitive(), (("a", "b"), ("b", "c"))) == set()


def test_ring_acyclic():
    v = ev(C.ring_acyclic(), (("a", "b"), ("b", "c"), ("c", "a")))
    assert v != set()                                         # a cycle exists somewhere
    assert ev(C.ring_acyclic(), (("a", "b"), ("b", "c"))) == set()


def test_frequency_bounds():
    pop = (("lab1", "s1"), ("lab1", "s2"), ("lab2", "s3"))
    v = ev(C.frequency([1], lo=2), pop)                       # each lab needs >= 2 students
    assert v == {("lab2", "s3")}
    v = ev(C.frequency([1], hi=1), pop)                       # each lab at most once
    assert v == {("lab1", "s1"), ("lab1", "s2")}
    assert ev(C.frequency([1], lo=1, hi=2), pop) == set()


def test_external_uniqueness_over_the_natural_join():
    # LabSession is for Subject (functional); LabSession is assigned to Student (m:n);
    # externally: each Student gets at most one session PER SUBJECT (book Fig. 10.21)
    D = ast.cell("LabSession_is_for_Subject", to_lam((("lab1", "CS1"), ("lab2", "CS1"))))
    Dw = L.SEQ(L.CONS(D)(L.NIL))
    c = C.scoped_external_uniqueness("LabSession_is_for_Subject", [3, 2])
    assigned = (("lab1", "stu1"), ("lab2", "stu1"))           # stu1 twice for CS1: violation
    v = set(from_lam(apply(c, L.SEQ(L.CONS(to_lam(assigned))(L.CONS(Dw)(L.NIL))))))
    assert v == {("lab1", "stu1", "CS1"), ("lab2", "stu1", "CS1")}
    ok = (("lab1", "stu1"), ("lab2", "stu2"))
    v2 = set(from_lam(apply(c, L.SEQ(L.CONS(to_lam(ok))(L.CONS(Dw)(L.NIL))))))
    assert v2 == set()


RING_MODEL = """Person is an entity type.
Person is parent of Person.
Person is parent of Person is acyclic.
"""


def test_ring_marker_parses_and_enforces():
    D, rep = forml.compile_model(RING_MODEL)
    assert rep["unparsed"] == []
    val = forml.validate_for("Person_is_parent_of_Person", D)
    from pyarest import defs
    cyc = to_lam((("a", "b"), ("b", "a")))
    with defs.step(D):
        _p, v, flag = from_lam(apply(val, L.SEQ(L.CONS(cyc)(L.CONS(D)(L.NIL)))))
    assert flag == "T" and len(v) > 0                         # alethic acyclicity fires


FREQ_MODEL = """Panel is an entity type.
Expert is an entity type.
Expert is on Panel.
In each population of Expert is on Panel, each Panel combination occurs at least 2 times.
"""


def test_frequency_reading_parses_and_enforces():
    D, rep = forml.compile_model(FREQ_MODEL)
    assert rep["unparsed"] == []
    val = forml.validate_for("Expert_is_on_Panel", D)
    from pyarest import defs
    pop = to_lam((("e1", "p1"),))                             # p1 has only one member
    with defs.step(D):
        _p, v, flag = from_lam(apply(val, L.SEQ(L.CONS(pop)(L.CONS(D)(L.NIL)))))
    assert flag == "T" and set(v) == {("e1", "p1")}


SUB_MODEL = """Patient is an entity type.
Woman is an entity type.
Woman is a subtype of Patient.
Each Woman had exactly one Pregnancy Count.
Pregnancy Count is a value type.
"""


def test_subtype_parses_absorbs_and_enforces():
    D, rep = forml.compile_model(SUB_MODEL)
    assert rep["unparsed"] == []
    Dpy = from_lam(D)
    assert any(c[:2] == ("CELL", "subtype") and ("Woman", "Patient") in c[2]
               for c in Dpy if isinstance(c, tuple) and len(c) == 3)
    # RMAP step 0: the subtype's functional fact types absorb into the TOP supertype's table
    part = system.rmap_partition(D)
    assert part["Woman_had_Pregnancy_Count"] == "Patient"
    # membership entailment: a Woman instance not among Patients violates
    D = apply(ast.Store("Woman"), L.SEQ(L.CONS(to_lam((("w1",),)))(L.CONS(D)(L.NIL))))
    D = apply(ast.Store("Patient"), L.SEQ(L.CONS(to_lam(()))(L.CONS(D)(L.NIL))))
    val = forml.validate_for("Woman", D)
    from pyarest import defs
    with defs.step(D):
        _p, v, flag = from_lam(apply(val, L.SEQ(L.CONS(to_lam((("w1",),)))(L.CONS(D)(L.NIL)))))
    assert flag == "T" and set(v) == {("w1",)}
