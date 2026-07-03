"""Constraint clause resolution must reach the DECLARED fact type when the reading
carries an article. The rule path learned this already (_SOME strips only some/that:
"'a' is predicate text ('is a parent of'), never stripped"), but the constraint-
clause path stripped every quantifier word including a/an, so "that Employee is a
manager" resolved to Employee_is_manager while the declaration founded
Employee_is_a_manager, and the exclusion attached to cells that do not exist —
a silently unenforced constraint. Resolution now prefers the minimal strip
(some/that/each/no) when it hits a declared fact type, falling back to the full
strip (which itself prefers a declared hit) so article-free models keep their ids."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import ast, forml, defs, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _check(val, pop, D):
    with defs.step(D):
        _p, v, flag = from_lam(apply(val, L.SEQ(L.CONS(to_lam(pop))(L.CONS(D)(L.NIL)))))
    return set(v), flag


MODEL = """Employee(.nr) is an entity type.
Employee is a manager.
Employee is a clerk.
For each Employee, at most one of the following holds: that Employee is a manager; that Employee is a clerk.
"""


def test_article_clauses_resolve_to_the_declared_fact_types():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    rows = [r for r in system._pop_rows(D, "constraint") if r[1] == "exclusion"]
    assert len(rows) == 1
    assert set(rows[0][3]) == {"Employee_is_a_manager", "Employee_is_a_clerk"}


def test_the_exclusion_actually_enforces_across_the_declared_cells():
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.Store("Employee_is_a_clerk"), S(to_lam((("e1",),)), D))
    val = forml.validate_for("Employee_is_a_manager", D)
    v, flag = _check(val, (("e1",),), D)                      # e1 already a clerk
    assert v and flag == "T"                                  # the family excludes


def test_article_free_clauses_keep_their_ids():
    M2 = ("Message is an entity type.\nRep is an entity type.\n"
          "Message matches Rep.\nMessage is sent by Rep.\n"
          "If some Message matches some Rep then that Message is sent by that Rep.")
    D, rep = forml.compile_model(M2)
    assert rep["unparsed"] == []
    rows = [r for r in system._pop_rows(D, "constraint") if r[1] == "subset"]
    assert rows[0][2] == "Message_matches_Rep"
    assert rows[0][3] == "Message_is_sent_by_Rep"
