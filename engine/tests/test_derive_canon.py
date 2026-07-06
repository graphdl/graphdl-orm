"""Wave S4: derive_S from the shared source, the paper's Def. derive verbatim —
derive = lfp(F_S), F_S(P) = P ∪ {heads derivable from P by one rule} — as a naive
WHILE to the fixed point over ⟨rules, D⟩, each round folding every rule's head
union through the store. Knaster-Tarski gives the lfp; Lemma finiteness bounds the
rounds; rules are positive and monotone. run_rules stays the production path (the
semi-naive frontier loop is the optimization, Bancilhon-Ramakrishnan), and the
canonical derive is the REFERENCE it must equal on the positive closure, gated here
on the transitive-closure and union-head models. nav_of and links_of ride along as
the last small builders (Thm. hateoas's navigation leg)."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import ast, defs, forml, system
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


CLOSURE = """Person(.id) is an entity type.
Person is a parent of Person.
Person is an ancestor of Person.
Person1 is an ancestor of Person2 if Person1 is a parent of Person2.
Person1 is an ancestor of Person3 if Person1 is a parent of Person2 and Person2 is an ancestor of Person3.
"""


def test_canonical_derive_reaches_the_same_lfp_as_run_rules():
    D, rep = forml.compile_model(CLOSURE)
    assert rep["rule_diagnostics"] == []
    D = apply(ast.Store("Person_is_a_parent_of_Person"),
              S(to_lam((("a", "b"), ("b", "c"), ("c", "d"))), D))
    rules = to_lam(tuple((rid, h) for (rid, h) in
                         ((r[0], r[1]) for r in system._pop_rows(D, "ruleDerives"))))
    with defs.step(D):
        D_canon = apply(apply(A("system:derive"), rules), D)
    D_prod = system.run_rules(D)
    want = {("a", "b"), ("b", "c"), ("c", "d"), ("a", "c"), ("b", "d"), ("a", "d")}
    got_c = _cell(from_lam(D_canon), "Person_is_an_ancestor_of_Person")
    got_p = _cell(from_lam(D_prod), "Person_is_an_ancestor_of_Person")
    assert got_c == got_p == want


def test_nav_and_links_come_from_the_canon():
    P = (("o1", "a"), ("o2", "b"), ("o1", "c"))
    with defs.step(L.SEQ(L.NIL)):
        nav = apply(A("system:nav_of"), A(1))
        got = from_lam(apply(nav, to_lam(P)))
        want = from_lam(apply(system.nav_of(1), to_lam(P)))
    assert got == want
    assert set(got) == {("o1", "a"), ("o1", "c")}             # the head entity's facts
