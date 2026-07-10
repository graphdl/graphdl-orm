"""#18: the ring-constraint reading handler, canonized — the first constraint-family
translator and the first with a non-phi objs half. The _COOK boundary resolves the
reading, so groups arrive as ⟨decl_rows, cid, kind_tag, ft, builder_name⟩; the canon
twin appends the constraint row (apndr) and emits ⟨cid, (builder : ⟨1,2⟩)⟩ — the
constraint OBJECT is the canonical builder applied through DEFS, which is exactly what
host C.ring_* does. The obj is a lambda, so the twin certifies it EXTENSIONALLY:
both objs applied to the same sample population answer the same violations."""
import pyarest.prims  # noqa: F401
from pyarest import canon, compiler
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

canon.load_all()

# cooked groups: ⟨decl_rows, cid, kind_tag, ft, builder_name⟩
COOKED = [
    (((("factType", ("X_likes_X", "{0} likes {1}")),
       ("role", ("X_likes_X.1", "X_likes_X", 1, "X")),
       ("role", ("X_likes_X.2", "X_likes_X", 2, "X")))),
     "X_likes_X_ring_irreflexive", "ring_irreflexive", "X_likes_X",
     "constraints:ring_irreflexive"),
    ((), "R_ring_acyclic", "ring_acyclic", "R", "constraints:ring_acyclic"),
]

# a population with one reflexive row — irreflexive must flag ⟨a,a⟩
POP = (("a", "a"), ("a", "b"), ("b", "a"))


def _pair(o):
    r = from_lam(o)
    return r


def _canon(groups):
    r = R(A("system:h_ring"), to_lam((groups, (), "alethic")))
    asserts = [(x[0], tuple(x[1])) for x in from_lam(R(A(1), r))]
    return asserts, R(A(2), r)


def _host(groups):
    a, o = compiler._h_ring(groups, None, "alethic")
    return [(c, tuple(row)) for c, row in a], o


def test_ring_asserts_twin_host():
    for g in COOKED:
        ca, _cobjs = _canon(g)
        ha, _hobjs = _host(g)
        assert ca == ha, g


def test_ring_asserts_shape():
    ca, _ = _canon(COOKED[0])
    assert ca[-1] == ("constraint",
                      ("X_likes_X_ring_irreflexive", "ring_irreflexive",
                       "X_likes_X", "alethic"))
    assert ca[0] == ("factType", ("X_likes_X", "{0} likes {1}"))


def test_ring_obj_extensional():
    g = COOKED[0]
    cobjs = _canon(g)[1]
    hobjs = _host(g)[1]
    # one obj each: ⟨cid, checker⟩ / [(cid, checker)]
    c_first = from_lam(R(A(1), R(A(1), cobjs)))
    assert c_first == hobjs[0][0] == "X_likes_X_ring_irreflexive"
    c_chk = R(A(2), R(A(1), cobjs))
    h_chk = hobjs[0][1]
    c_ans = from_lam(R(c_chk, to_lam(POP)))
    h_ans = from_lam(R(h_chk, to_lam(POP)))
    assert c_ans == h_ans
    # non-vacuous: the reflexive pair is flagged
    assert c_ans and ("a", "a") in [tuple(x) for x in c_ans]
