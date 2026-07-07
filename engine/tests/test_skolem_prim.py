"""The skolem primitive (task-970's value-invention leaf, the 0.9.0
mapping): applied to a sequence of atom values (the FRONTIER), answers
the atom 've_' + fnv1a64-hex of the values joined by '|'. Deterministic
and frontier-keyed — same frontier, same id; the semi-oblivious chase's
'one labelled null per frontier tuple' — which is what makes an
existential head an ordinary OWNED sweep head (re-derivation reproduces
the ids, set semantics dedup). Non-sequence input answers Bottom.
Boundary-op discipline: this python prim is the reference; the peer
kernels twin it through the case table (the lex/implode/slug drill)."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, from_lam
from pyarest.reduce import apply


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _skolem(*vals):
    return from_lam(apply(A("skolem"), _S(*[A(v) for v in vals])))


def test_deterministic_and_frontier_keyed():
    a = _skolem("menu", "t1")
    b = _skolem("menu", "t1")
    c = _skolem("menu", "t2")
    assert a == b, "same frontier must answer the same id"
    assert a != c, "distinct frontiers must answer distinct ids"
    assert isinstance(a, str) and a.startswith("ve_") and len(a) == 19
    assert all(ch in "0123456789abcdef" for ch in a[3:])


def test_the_reference_vector_pins_the_hash():
    # fnv1a64 over the bytes of 'menu|t1' (standard offset/prime), hex —
    # the cross-host case rows pin this exact value
    import json
    got = _skolem("menu", "t1")
    want = "ve_"
    h = 14695981039346656037
    for byte in "menu|t1".encode("utf-8"):
        h = ((h ^ byte) * 1099511628211) % (1 << 64)
    want += format(h, "016x")
    assert got == want, json.dumps({"got": got, "want": want})


def test_bottom_on_shape_mismatch():
    out = from_lam(apply(A("skolem"), A("not-a-sequence")))
    assert out == "⊥"
