"""The static condition that discharges Lemma finiteness' hypothesis: recursion through
the rule dependency graph is admitted (heads are range-restricted by construction, so the
fixpoint is over a finite atom domain and terminates), but no dependency CYCLE may pass
through value invention — a rule whose definition applies a registered (boundary)
function can introduce individuals drawn from no stored population, unboundedly. Acyclic
invention stays admissible. Definitions are cells, so the check reads rule bodies from D
like everything else."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


MODEL = """Party is an entity type.
Person is an entity type.
Person is a subtype of Party.
State Machine Definition 'Party' is for Noun 'Party'.
Status 'New' is initial in State Machine Definition 'Party'.
"""


def _add_rows(D, name, rows):
    old = tuple(tuple(r) for r in system._pop_rows(D, name))
    return apply(ast.Store(name), S(to_lam(old + rows), D))


def test_pure_recursion_is_admitted():
    D, _ = forml.compile_model(MODEL)
    D = system.governance_rules(D)                            # governedBy reads governedBy:
    assert system.finiteness_check(D) == []                   # a cycle, pure, admitted


def test_a_cycle_through_a_boundary_def_is_flagged():
    system._register_cellkey()                                # a registered (boundary) op
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.DefineIn("pure_rule", system.compile_rule(["A"], [1])), D)
    D = apply(ast.DefineIn("inventive_rule", S(A("COMP"), A("cellkey"), A(1))), D)
    D = _add_rows(D, "ruleReads", (("pure_rule", "A"), ("inventive_rule", "B")))
    D = _add_rows(D, "ruleDerives", (("pure_rule", "B"), ("inventive_rule", "A")))
    assert system.finiteness_check(D) == ["inventive_rule"]   # invention on a cycle


def test_acyclic_invention_is_admitted():
    system._register_cellkey()
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.DefineIn("keyed_view", S(A("COMP"), A("cellkey"), A(1))), D)
    D = _add_rows(D, "ruleReads", (("keyed_view", "C"),))
    D = _add_rows(D, "ruleDerives", (("keyed_view", "E"),))   # E never reaches back to C
    assert system.finiteness_check(D) == []                   # finite composition: fine
