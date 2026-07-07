"""Existential (TGD) heads under 0.9.0 (task-970's surface, the skolem
slice's compiler half): a subscripted head variable that never appears
in the body is EXISTENTIAL — the compiler emits its projection entry as
⟨COMP, skolem, ⟨CONS, frontier selectors⟩⟩ over the joined row (the
frontier = the body-bound variables, first-appearance order), so the
rule mints one deterministic fresh id per frontier binding. Same
frontier, same id: two rules with the same body SHARE their fresh
entity (the multi-consequent E), and a re-derivation reproduces the
population byte-identically — the semi-oblivious chase as an ordinary
derivation-owned sweep."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system
from pyarest.engine import _pop_rows

MODEL = """View(.id) is an entity type.
Transition(.id) is an entity type.
View Element(.id) is an entity type.
Component Role is a value type.
View offers Transition.
View Element renders Transition. *
View Element has Component Role. *

* View Element1 renders Transition1 iff View1 offers Transition1.

* View Element1 has Component Role 'button' iff View1 offers Transition1.

View 'v1' offers Transition 't1'.
View 'v1' offers Transition 't2'.
"""


def test_existential_heads_mint_frontier_stable_elements():
    D, rep = forml.compile_model(MODEL)
    assert rep["rule_diagnostics"] == [], rep["rule_diagnostics"]
    D = system.run_rules(D)
    rend = {tuple(str(x) for x in r)
            for r in _pop_rows(D, "View_Element_renders_Transition")}
    assert {r[1] for r in rend} == {"t1", "t2"}
    ids = {r[0] for r in rend}
    assert len(ids) == 2, "one fresh element per frontier binding"
    assert all(i.startswith("ve_") and len(i) == 19 for i in ids)
    role = {tuple(str(x) for x in r)
            for r in _pop_rows(D, "View_Element_has_Component_Role")}
    # the SHARED existential: both rules carry the same body, hence the
    # same frontier, hence the SAME fresh ids — no parser plumbing needed
    assert role == {(i, "button") for i in ids}


def test_rederivation_is_idempotent_no_duplicate_elements():
    D, _rep = forml.compile_model(MODEL)
    D = system.run_rules(D)
    before = {tuple(r) for r in _pop_rows(D, "View_Element_renders_Transition")}
    D = system.run_rules(D)
    after = {tuple(r) for r in _pop_rows(D, "View_Element_renders_Transition")}
    assert after == before, "same frontier must reproduce the same ids"
