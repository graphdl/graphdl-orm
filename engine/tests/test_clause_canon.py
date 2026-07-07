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


def test_the_canonical_clause_resolution_twins_the_python_over_the_corpus():
    """The strongest oracle: every constraint clause the four handler
    families extract from shared/base answers the SAME fact-type id
    through system:clause_ft as through compiler._clause_ft. The clause
    texts replicate each handler's own extraction (compiler.py 874-906)."""
    import os
    from pyarest import forml
    from pyarest.compiler import _clause_ft, _Known
    root = os.path.join(os.path.dirname(os.path.dirname(
        os.path.abspath(__file__))), "shared", "base")
    text = "\n\n".join(open(os.path.join(root, f), encoding="utf-8").read()
                       for f in sorted(os.listdir(root)) if f.endswith(".md"))
    stmts = forml.statements(text)
    known = forml._known(stmts)
    names = sorted(set(known))
    fts = getattr(known, "fts", None) or set()
    clauses = []
    for s in stmts:
        kind, g, _m = forml.analyze(s)
        if kind == "set_comparison":
            clauses += [c.strip() for c in g[2].split(";") if c.strip()]
        elif kind == "disjunctive_mandatory":
            body = g[-1]
            subj, rest = (forml._subject(body, known) if len(g) == 1
                          else (forml._subject(g[0], known)[0], body))
            clauses += [subj + " " + c.strip()
                        for c in rest.split(" or ") if c.strip()]
        elif kind == "subset":
            conseq, _, _w = g[1].partition(" where ")
            clauses += [g[0].strip(), conseq.strip()]
        elif kind == "equality":
            clauses += [g[0].strip(), g[1].strip()]
    checked = 0
    for c in clauses:
        want = _clause_ft(c, known)
        got = _clause(c, names, sorted(fts))
        assert got == want, (c, got, want)
        checked += 1
    assert checked >= 8                                # the corpus is real
