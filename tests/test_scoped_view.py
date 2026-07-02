"""The RMAP plan's recorded dependency, closed: scoped constraints read ABSORBED fact
types through the view (index + dynamic fetch), not the per-fact-type cell. With the
partition, validate_for rebuilds the affected scoped objects over ftpop_expr; without
it, the per-fact-type cell is empty under the routed layout and the constraint reports
spuriously. The discriminator: a routed write satisfies the mandatory constraint only
through the view."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, defs, forml, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


MODEL = """Person is an entity type.
Passport is an entity type.
Person holds Passport.
Each Person holds at most one Passport.
Each Person holds some Passport.
"""


def test_scoped_mandatory_reads_through_the_view():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    part = system.rmap_partition(D)
    assert part["Person_holds_Passport"] == "Person"          # functional: absorbed
    D = apply(ast.Store("Person"), S(to_lam((("p1",), ("p2",))), D))
    D = apply(A(2), system.create(D, "Person_holds_Passport", to_lam(("p1", "pp1"))))
    ents = (("p1",), ("p2",))
    vo = forml.validate_for("Person", D, partition=part)
    with defs.step(D):
        (_p, V, _f) = from_lam(apply(vo, S(to_lam(ents), D)))
    assert ("p2",) in set(V) and ("p1",) not in set(V)        # p1 satisfied via the VIEW
    vo0 = forml.validate_for("Person", D)
    with defs.step(D):
        (_p, V0, _f0) = from_lam(apply(vo0, S(to_lam(ents), D)))
    assert ("p1",) in set(V0)                                 # the per-ft cell cannot see it
