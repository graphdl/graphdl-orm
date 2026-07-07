"""Constraint-clause resolution as a canonical object (the L1 arc's next
pocket after the reading scan). system:clause_ft — ⟨text, knowns, stop,
declared⟩ → the fact-type id a quantified constraint clause references:
the MINIMAL quantifier strip (some/that/each/no) resolves first and wins
when its id is DECLARED (the article lesson: 'is a manager' declares
Employee_is_a_manager, and stripping the article resolved the clause to a
cell that does not exist — a silently unenforced constraint); otherwise
the full strip (+ an/a) answers regardless. The Python _clause_ft
(compiler.py) is the behavioral spec and becomes a thin caller; the
constraint clauses of the shared/base corpus are the twin oracle."""
import pyarest.prims  # noqa: F401
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest.reduce import apply

STOP = ("If", "When", "Then", "That", "This", "An", "A", "The", "Each",
        "Some", "No", "Every", "Not", "It", "There", "Once", "For", "In",
        "Of", "To", "On", "At", "By", "With", "And", "Or", "Only")


def _clause(text, knowns, declared):
    return from_lam(apply(A("system:clause_ft"),
                          to_lam((text, tuple(knowns), STOP,
                                  tuple(declared)))))


def test_a_quantified_clause_drops_the_quantifier():
    assert _clause("Ticket has some Status", ("Ticket", "Status"),
                   ("Ticket_has_Status",)) == "Ticket_has_Status"


def test_the_article_stays_when_the_articled_id_is_declared():
    # the article lesson: 'is a manager' is predicate text; the minimal
    # strip keeps it and the declared id wins
    assert _clause("Employee is a manager", ("Employee",),
                   ("Employee_is_a_manager",)) == "Employee_is_a_manager"


def test_the_full_strip_answers_when_the_minimal_id_is_undeclared():
    # no declared hit under the minimal strip: the full strip's id answers
    # regardless of declaration (article-free models keep their ids)
    assert _clause("Employee is a manager", ("Employee",),
                   ()) == "Employee_is_manager"
