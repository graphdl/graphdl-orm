"""validate_S from the shared source (Def. Command / Def. Violation): ⟨P, D⟩ maps to
⟨P, V, alethicViolated⟩ where V unions every family's violations and only the ALETHIC
subset raises the commit-blocking flag ("an alethic c rejects the commit when V_c is
nonempty; a deontic c warns and commits"). The canonical builder takes the record
⟨local, alethic?, scoped, scoped_alethic?⟩ of OBJECT sequences: local constraints
compose with the target-population selector, scoped ones consume ⟨P, D⟩ whole, and an
absent alethic slot defaults to the whole family (the empty-but-provided slot is the
deliberately-deontic case, so absence and emptiness are distinct encodings)."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import defs, system, theta
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


NONEMPTY = S(A("COMP"), A("theta:Filter"), A("id"))           # placeholder; not used
V_ALL = A("id")                                               # local: V = P itself
V_NONE = S(A("CONST"), to_lam(()))                            # local: no violations
SCOPED_D = A(2)                                               # scoped: V = D (visibly cross-cell)


def _slot(v):
    return to_lam(()) if v is None else S(v)


def _run(record, P, D, kwargs):
    with defs.step(L.SEQ(L.NIL)):
        canon = apply(A("system:validate_of"), record)
        got = from_lam(apply(canon, S(to_lam(P), D)))
        want = from_lam(apply(system.validate_of(**kwargs), S(to_lam(P), D)))
    assert got == want, f"validate divergence: {got!r} != {want!r}"
    return got


def test_alethic_local_blocks_and_reports():
    got = _run(S(S(V_ALL), to_lam(()), to_lam(()), to_lam(())),
               (("x",),), to_lam(()),
               {"constraints": [V_ALL]})
    assert got == ((("x",),), (("x",),), "T")                 # P, V, flag


def test_deontic_local_reports_but_commits():
    # alethic PROVIDED and empty: the violation reports in V, the flag stays F
    got = _run(S(S(V_ALL), S(to_lam(())), to_lam(()), to_lam(())),
               (("x",),), to_lam(()),
               {"constraints": [V_ALL], "alethic": []})
    assert got == ((("x",),), (("x",),), "F")


def test_clean_population_neither_reports_nor_blocks():
    got = _run(S(S(V_NONE), to_lam(()), to_lam(()), to_lam(())),
               (("x",),), to_lam(()),
               {"constraints": [V_NONE]})
    assert got == ((("x",),), (), "F")


def test_scoped_families_consume_the_pair():
    D = to_lam((("sib", "row"),))
    got = _run(S(to_lam(()), to_lam(()), S(SCOPED_D), to_lam(())),
               (("p",),), D,
               {"constraints": [], "scoped": [SCOPED_D]})
    assert got[2] == "T" and got[1] == (("sib", "row"),)      # scoped saw D; default alethic


def test_empty_everything_is_the_stub():
    got = _run(S(to_lam(()), to_lam(()), to_lam(()), to_lam(())),
               (("p",),), to_lam(()),
               {"constraints": []})
    assert got == ((("p",),), (), "F")
