"""#18: the frequency-constraint reading handler, canonized — the second constraint-family
translator. Cooked groups ⟨cid, ft, roles, builder_operand⟩; the canon twin prepends the
constraint row to the per-role spans rows (α over distl — the first MAPPED row family) and
emits ⟨cid, (constraints:frequency : operand)⟩. Extensional obj certification like ring."""
import pyarest.prims  # noqa: F401
from pyarest import canon, compiler
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

canon.load_all()

# cooked: ⟨cid, ft, roles, builder_operand⟩ — operand ⟨roles, lo?, hi?⟩, absent = ()
COOKED = [
    ("R_freq", "R", (1,), ((1,), (), (1,))),          # at most 1 over role 1
    ("S_freq", "S", (1, 2), ((1, 2), (2,), (2,))),    # exactly 2 over roles 1,2
]

POP = (("a", "x"), ("a", "y"), ("b", "z"))            # key "a" occurs twice, "b" once


def _canon(groups):
    r = R(A("system:h_frequency"), to_lam((groups, (), "alethic")))
    asserts = [(x[0], tuple(x[1])) for x in from_lam(R(A(1), r))]
    return asserts, R(A(2), r)


def _host(groups):
    a, o = compiler._h_frequency(groups, None, "alethic")
    return [(c, tuple(row)) for c, row in a], o


def test_frequency_asserts_twin_host():
    for g in COOKED:
        assert _canon(g)[0] == _host(g)[0], g


def test_frequency_asserts_shape():
    ca, _ = _canon(COOKED[1])
    assert ca[0] == ("constraint", ("S_freq", "frequency", "S", "alethic"))
    assert ca[1:] == [("spans", ("S_freq", 1)), ("spans", ("S_freq", 2))]


def test_frequency_obj_extensional():
    g = COOKED[0]                                      # at most 1: key "a" violates
    cobjs, hobjs = _canon(g)[1], _host(g)[1]
    c_first = from_lam(R(A(1), R(A(1), cobjs)))
    assert c_first == hobjs[0][0] == "R_freq"
    c_ans = from_lam(R(R(A(2), R(A(1), cobjs)), to_lam(POP)))
    h_ans = from_lam(R(hobjs[0][1], to_lam(POP)))
    assert c_ans == h_ans
    assert c_ans, "the double-occurring key must be flagged (non-vacuous)"
