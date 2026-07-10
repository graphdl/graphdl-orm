"""#18 pilot: the compiler's objectification translator moved into system.canon.
An objectification reading's two capture groups <association, entity> give the entity
its ObjectType instance-of and the objectification link to its association:
  system:h_objectification : <groups, known, mod>
     -> <<<instanceOf,<entity,ObjectType>>,<objectification,<entity,association>>>, phi>
Doctrine (Samuel): even the compiler is canonized; DEFS override provides performance,
lambdas provide the execution lattice. The host _h_objectification stays a native
certified-equal override so compiles keep their speed. This test twins the two."""
import pyarest.prims  # noqa: F401
from pyarest import canon, compiler
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

canon.load_all()


def _canon(groups):
    r = from_lam(R(A("system:h_objectification"), to_lam((tuple(groups), (), ""))))
    asserts = [(x[0], tuple(x[1])) for x in r[0]]
    objs = list(r[1]) if len(r) > 1 else []
    return asserts, objs


def _host(groups):
    a, o = compiler._h_objectification(groups, None, None)
    return [(c, tuple(row)) for c, row in a], list(o)


def test_h_objectification_shape():
    g = ("Person plays Role", "Playing")
    assert _canon(g) == (
        [("instanceOf", ("Playing", "ObjectType")),
         ("objectification", ("Playing", "Person plays Role"))], [])


def test_h_objectification_twins_host():
    for g in [("Person plays Role", "Playing"),
              ("Company employs Person", "Employment"),
              ("A r B", "X")]:
        assert _canon(g) == _host(g)
