"""DEFS lives in D (audit C6 / Def. AREST): compiled definitions ride the transitioned
store in a DEFS cell. mu resolves a name FIRST against the step's D (bound by run/dispatch
and frozen for the step, Backus §14.6), then the process seed (the Stage-1 hand-seeded base
+ the registered boundary). Consequences under test: schema ingestion threads its compiled
objects into D (Cor. closure), sibling tenant stores carry their OWN DEFS (spec §4.5), and
a name absent from a store is unaddressable there (Prop. tenant)."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import reduce as R
from pyarest import ast, defs, forml


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _D(*cells):
    l = L.NIL
    for c in reversed(cells):
        l = L.CONS(c)(l)
    return L.SEQ(l)


BASE = lambda: _D(ast.cell("FILE", to_lam(())))


def test_a_name_in_the_stores_defs_cell_resolves_during_a_step():
    D = from_defs = R.apply(ast.DefineIn("shout", S(A("CONST"), A("LOUD"))), BASE())
    (o, _Dp) = from_lam(ast.run(to_lam(("f",)), D, derive_obj=A("shout")))
    assert o[0] == "LOUD"                                     # the step resolved shout from D


def test_step_defs_do_not_leak_outside_the_step():
    R.apply(ast.DefineIn("leaky", S(A("CONST"), A("X"))), BASE())
    assert from_lam(R.apply(A("leaky"), to_lam(("y",)))) == "⊥"   # not in the process store


def test_sibling_tenants_resolve_their_own_defs():
    D_a = R.apply(ast.DefineIn("greet", S(A("CONST"), A("from-A"))), BASE())
    D_b = R.apply(ast.DefineIn("greet", S(A("CONST"), A("from-B"))), BASE())
    (oa, _) = from_lam(ast.run(to_lam(("f",)), D_a, derive_obj=A("greet")))
    (ob, _) = from_lam(ast.run(to_lam(("f",)), D_b, derive_obj=A("greet")))
    assert (oa[0], ob[0]) == ("from-A", "from-B")             # same name, per-store meaning
    # a store WITHOUT the def cannot address it at all (Prop. tenant)
    assert from_lam(ast.run(to_lam(("f",)), BASE(), derive_obj=A("greet"))) == "⊥"


def test_both_paths_agree_on_step_defs():
    D = R.apply(ast.DefineIn("s2", S(A("CONS"), A(2), A(1))), BASE())
    x = to_lam(("a", "b"))
    with defs.step(D):
        assert from_lam(R.apply_lambda(A("s2"), x)) == from_lam(R.apply(A("s2"), x)) == ("b", "a")


def test_compile_defines_into_D_not_the_process_store():
    D, _rep = forml.compile_model("Each Student has at most one Email.")
    assert "Student_has_Email_uc" not in defs.compiled        # ingestion no longer mutates the seed
    viol = to_lam((("s1", "a"), ("s1", "b")))
    with defs.step(D):                                        # …but the name lives in THIS store
        v = from_lam(R.apply(A("Student_has_Email_uc"), viol))
    assert set(v) == {("s1", "a"), ("s1", "b")}
