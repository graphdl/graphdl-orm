"""The verify AUDIT as SHARED CANON — the canonicalization arc's last
host-only meaning pocket (2026-07-08, Samuel: finish the root and
canonicalization arcs). system:audit_heads answers WHICH heads must
reproduce (the destructive passes off the passHeads cell — sweep, dred,
aggwhole — plus keyed heads whose derivation kind is owned, kept to the
ruled ones); system:audit_recompute re-evaluates a head's rules over
the settled store (rule ids applied straight off ruleDerives — cells
resolve first, exactly as run_rules dispatches; an atom answer reads
as the empty contribution, python's unevaluable-rule guard);
system:audit_match is double set-inclusion between the stored cell and
the recomputation; system:verify_store maps the audit across the store.
protocol.Registry.verify is hereby the certified-equal override — its
counts stay host report decoration."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest import defs
from pyarest.lam import atom as A, from_lam
from pyarest.reduce import apply


def _ap(f, x, D):
    # rule ids resolve through rho — the ambient step context — exactly
    # as run_rules and Registry.verify evaluate them (cells first)
    with defs.step(D):
        return from_lam(apply(A(f), x))


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _copy_rule(src):
    # the compiled one-atom copy rule, canonical parts only (derive.rs's
    # own fixture shape): obj : D = (ast:FetchPop : src) : D
    return _S(A("COMP"), A("apply"),
              _S(A("CONS"),
                 _S(A("COMP"), A("apply"),
                    _S(A("CONS"), _S(A("CONST"), A("ast:FetchPop")),
                       _S(A("CONST"), A(src)))),
                 A("id")))


def _DA():
    """Two ruled sweep heads: Good's stored cell IS its rule's answer
    (a copy of Src); Bad's stored cell was tampered (an extra row the
    rule does not derive). A keyed OWNED head (Own, copy of Src2)
    audits too; a keyed UNOWNED head (Free) does not. Unruled heads
    never audit."""
    return _S(
        _S(A("CELL"), A("passHeads"),
           _S(_S(A("sweep"), A("Good")),
              _S(A("sweep"), A("Bad")),
              _S(A("keyed"), A("Own")),
              _S(A("keyed"), A("Free")),
              _S(A("sweep"), A("Unruled")))),
        _S(A("CELL"), A("derivation"),
           _S(_S(A("Own"), A("fully-derived")),
              _S(A("Free"), A("asserted")))),
        _S(A("CELL"), A("ruleDerives"),
           _S(_S(A("r_good"), A("Good")),
              _S(A("r_bad"), A("Bad")),
              _S(A("r_own"), A("Own")))),
        _S(A("CELL"), A("Src"), _S(_S(A("a"), A("1")), _S(A("b"), A("2")))),
        _S(A("CELL"), A("Src2"), _S(_S(A("c"), A("3")))),
        _S(A("CELL"), A("Good"), _S(_S(A("a"), A("1")), _S(A("b"), A("2")))),
        _S(A("CELL"), A("Bad"),
           _S(_S(A("a"), A("1")), _S(A("b"), A("2")), _S(A("z"), A("9")))),
        _S(A("CELL"), A("Own"), _S(_S(A("c"), A("3")))),
        _S(A("CELL"), A("r_good"), _copy_rule("Src")),
        _S(A("CELL"), A("r_bad"), _copy_rule("Src")),
        _S(A("CELL"), A("r_own"), _copy_rule("Src2"))
    )


def test_audit_heads_selects_destructive_and_owned_keyed_ruled():
    D = _DA()
    got = _ap("system:audit_heads", D, D)
    assert got == ("Good", "Bad", "Own")


def test_audit_match_confirms_and_refutes():
    D = _DA()
    assert _ap("system:audit_match", _S(A("Good"), D), D) == "T"
    assert _ap("system:audit_match", _S(A("Bad"), D), D) == "F"


def test_verify_store_maps_the_audit():
    D = _DA()
    got = _ap("system:verify_store", D, D)
    assert got == (("Good", "T"), ("Bad", "F"), ("Own", "T"))
