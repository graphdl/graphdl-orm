"""Scoped validation (audit C3): no parsed constraint silently drops from enforcement.
Cross-cell families — mandatory, subset, equality, set-comparison, value — enforce over
⟨P, D⟩: P is the target cell's post-derive population, sibling cells are fetched from the
frozen D. validate_for assembles them from M's constraint facts alone (no host filter)."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, defs, forml, system
from pyarest.reduce import apply


def _D(*cells):
    l = L.NIL
    for c in reversed(cells):
        l = L.CONS(c)(l)
    return L.SEQ(l)


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return None


MODEL = """Student is an entity type.
Email is a value type.
Each Student has some Email.
"""


def _with_pops(D, **pops):
    for name, pop in pops.items():
        D = apply(ast.Store(name), L.SEQ(L.CONS(to_lam(pop))(L.CONS(D)(L.NIL))))
    return D


def test_mandatory_enforces_through_validate_for():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    D = _with_pops(D, Student=(("s1",), ("s2",)), Student_has_Email=(("s1", "a@x"),))
    val = forml.validate_for("Student_has_Email", D)
    with defs.step(D):
        _p, v, flag = from_lam(apply(val, L.SEQ(L.CONS(to_lam((("s1", "a@x"),)))(L.CONS(D)(L.NIL)))))
    assert set(v) == {("s2",)}                                # s2 plays no has-Email fact
    assert flag == "T"                                        # alethic mandatory would block


def test_mandatory_blocks_the_create_that_leaves_an_entity_bare():
    D, _ = forml.compile_model(MODEL)
    D = _with_pops(D, Student=(("s1",),), Student_has_Email=())
    # committing a NEW student (s2) while s1 still has no Email: the mandatory violation
    # (computed over ⟨P'', D⟩) blocks the commit and D is unchanged
    val = forml.validate_for("Student", D)
    (o, Dp) = from_lam(ast.run(to_lam(("s2",)), D, cell_name="Student", validate_obj=val))
    assert _cell(Dp, "Student") == {("s1",)}                  # refused: bare students exist


def test_subset_clauses_resolve_to_fact_types_and_enforce():
    model = ("Message is an entity type.\nRep is an entity type.\n"
             "Message matches Rep.\nMessage is sent by Rep.\n"
             "If some Message matches some Rep then that Message is sent by that Rep.")
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    D = _with_pops(D, Message_is_sent_by_Rep=(("m1", "r1"),))
    val = forml.validate_for("Message_matches_Rep", D)
    matches = (("m1", "r1"), ("m2", "r2"))                    # m2/r2 not in sent-by
    with defs.step(D):
        _p, v, flag = from_lam(apply(val, L.SEQ(L.CONS(to_lam(matches))(L.CONS(D)(L.NIL)))))
    assert set(v) == {("m2", "r2")} and flag == "T"


def test_exclusion_enforces_through_validate_for_on_a_clause_cell():
    model = ("Message is an entity type.\nPhone is a value type.\nEmail is a value type.\n"
             "Message is with Phone.\nMessage is with Email.\n"
             "For each Message, at most one of the following holds: "
             "that Message is with some Phone; that Message is with some Email.")
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    D = _with_pops(D, Message_is_with_Email=(("m1", "e1"),))
    val = forml.validate_for("Message_is_with_Phone", D)      # commit lands on the Phone clause
    with defs.step(D):
        _p, v, _f = from_lam(apply(val, L.SEQ(L.CONS(to_lam((("m1", "p1"),)))(L.CONS(D)(L.NIL)))))
    assert set(v) == {("m1", "Message_is_with_Phone"), ("m1", "Message_is_with_Email")}


def test_value_constraint_enforces_on_the_types_cell():
    D, rep = forml.compile_model("Grade is a value type.\nThe possible values of Grade are A, B, C.")
    assert rep["unparsed"] == []
    val = forml.validate_for("Grade", D)
    with defs.step(D):
        _p, v, flag = from_lam(apply(val, L.SEQ(L.CONS(to_lam((("A",), ("F",))))(L.CONS(D)(L.NIL)))))
    assert set(v) == {("F",)} and flag == "T"


def test_no_silent_enforcement_filter_remains():
    assert not hasattr(forml, "_ENFORCEABLE")                 # the host drop-table is gone
