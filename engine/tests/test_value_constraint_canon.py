"""#18: the value-constraint reading handler, canonized — third constraint-family
translator. Cooked groups ⟨name, spec, cid, builder_name, operand⟩ (the spec parse is
the string boundary); the canon twin emits the two rows and ⟨cid, (builder : operand)⟩
through DEFS. Extensional obj certification over range and enumeration builders."""
import pyarest.prims  # noqa: F401
from pyarest import canon, compiler
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

canon.load_all()

RANGE = ("Age", "[0..120]", "Age_vc",
         "constraints:value_range", (1, (0, "F"), (120, "F")))
ENUM = ("Room", "'kitchen', 'bath'", "Room_vc",
        "constraints:value_enumeration", (1, ("kitchen", "bath")))


def _canon(groups):
    r = R(A("system:h_value_constraint"), to_lam((groups, (), "alethic")))
    asserts = [(x[0], tuple(x[1])) for x in from_lam(R(A(1), r))]
    return asserts, R(A(2), r)


def _host(groups):
    a, o = compiler._h_value_constraint(groups, None, "alethic")
    return [(c, tuple(row)) for c, row in a], o


def test_value_constraint_asserts_twin_host():
    for g in (RANGE, ENUM):
        assert _canon(g)[0] == _host(g)[0], g


def test_value_constraint_shape():
    ca, _ = _canon(RANGE)
    assert ca == [("valueConstraint", ("Age", "[0..120]", "alethic")),
                  ("constraint", ("Age_vc", "value", "Age", "alethic"))]


def test_value_constraint_obj_extensional():
    for g, pop in ((ENUM, (("kitchen",), ("attic",))),
                   (RANGE, ((150,), (30,)))):
        cobjs, hobjs = _canon(g)[1], _host(g)[1]
        assert from_lam(R(A(1), R(A(1), cobjs))) == hobjs[0][0] == g[2]
        c_ans = from_lam(R(R(A(2), R(A(1), cobjs)), to_lam(pop)))
        h_ans = from_lam(R(hobjs[0][1], to_lam(pop)))
        assert c_ans == h_ans
        assert c_ans, ("the out-of-bounds value must be flagged", g[0])


def test_value_spec_parse():
    # the boundary parse picks the right builder + encoding
    assert compiler._value_spec("[0..120]") == (
        "constraints:value_range", (1, (0, "F"), (120, "F")))
    assert compiler._value_spec("above 5") == (
        "constraints:value_range", (1, (5, "T"), ()))
    assert compiler._value_spec("'kitchen', 'bath'") == (
        "constraints:value_enumeration", (1, ("kitchen", "bath")))
