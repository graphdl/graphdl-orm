"""Modality (alethic vs deontic) and value constraints, per the constraint verbalization paper.
Alethic violations block commit; deontic violations flag but commit (AREST Def. Violation).
Value constraints cover enumerations and open/closed ranges."""
from pyarest import from_lam, to_lam, apply
from pyarest.lam import atom as A
import pyarest.lam as L
import pyarest.prims  # noqa: F401
from pyarest import ast, system, forml
from pyarest import constraints as C


def _D(pop):
    return L.SEQ(L.CONS(ast.cell("FILE", to_lam(pop)))(L.NIL))

def _file(Dpy):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", "FILE"):
            return set(c[2])
    return None


# ---- modality parsing ----
def test_modality_is_tagged():
    # kinds are the GRAMMAR's after the seed deletion; the seed still
    # tags MODALITY (its bootstrap job), asserted on the catch-all kind
    assert forml.analyze("Each Student has at most one Email.")[2] == "alethic"
    kind, _g, mod = forml.analyze("It is obligatory that each Student has at most one Email.")
    assert kind == "uniqueness" and mod == "deontic"
    assert forml.analyze("It is possible that more than one Student has the same Email.")[0] == "possibility"
    kind, _g, mod = forml.analyze("It is forbidden that each Student has more than one Email.")
    assert mod == "deontic"


# ---- #34: leading-marker derivation rules (Sherlock deontic core) ----
import pytest


def test_leading_plus_if_rule_is_not_the_factype_catchall():
    # #34 (FIXED): a leading '+' marks a STORED derivation; '+ <head> if <body>'
    # classifies in the trailing-if family, NOT the fact_type_reading catch-all
    # (which dequoted the whole line into a phantom instance fact). subset_trailing
    # (compiler.py:341) now captures the optional leading marker, mirroring
    # rule_iff (317). Sherlock evidence.md:54, reasoning.md:85.
    kind, g, _mod = forml.analyze(
        "+ Evidence has Evidence Weight 'Strong' if Evidence comes from some "
        "Evidence Source.")
    assert kind == "subset_trailing", (kind, g)
    # the marker is captured as group 1 (head/body follow), so the handler can
    # dispatch on the storage kind instead of the catch-all silently minting rows
    assert g[0] == "+", g
    assert g[1].startswith("Evidence has Evidence Weight"), g


# ---- the commit rule with modality (the load-bearing point) ----
def test_alethic_violation_blocks_commit():
    val = system.validate_modal([(C.uniqueness([1]), "alethic")])
    (o, Dp) = from_lam(ast.run(to_lam(("a", "z")), _D((("a", "x"),)), validate_obj=val))
    _p2, v = o
    assert set(v) == {("a", "x"), ("a", "z")}                # the violation is reported
    assert _file(Dp) == {("a", "x")}                          # and D is NOT committed

def test_deontic_violation_flags_but_commits():
    val = system.validate_modal([(C.uniqueness([1]), "deontic")])
    (o, Dp) = from_lam(ast.run(to_lam(("a", "z")), _D((("a", "x"),)), validate_obj=val))
    _p2, v = o
    assert set(v) == {("a", "x"), ("a", "z")}                # deontic violation still reported in V
    assert _file(Dp) == {("a", "x"), ("a", "z")}              # but D IS committed (deontic never blocks)


# ---- value constraints: enumeration + ranges ----
def test_value_enumeration():
    obj = forml._value_constraint("A, B, C")
    v = from_lam(apply(obj, to_lam((("A",), ("D",), ("B",), ("F",)))))
    assert set(v) == {("D",), ("F",)}                         # values outside the enumeration

def test_value_closed_range():
    obj = forml._value_constraint("[0..120]")
    v = from_lam(apply(obj, to_lam(((5,), (200,), (0,), (120,), (-1,)))))
    assert set(v) == {(200,), (-1,)}                          # outside [0,120]

def test_value_open_range():
    obj = forml._value_constraint("at least -273.15 and below 0")
    v = from_lam(apply(obj, to_lam(((-300.0,), (-100.0,), (0.0,), (5.0,)))))
    assert set(v) == {(-300.0,), (0.0,), (5.0,)}              # below -273.15, or >= 0 (hi is open)


# ---- modality read off M governs a compiled model's enforcement (Cor. closure) ----
def test_validate_for_reads_modality_from_M():
    from pyarest import defs

    def _pd(p, D):                                            # validate consumes ⟨P, D⟩
        return L.SEQ(L.CONS(p)(L.CONS(D)(L.NIL)))

    viol = to_lam((("s1", "a"), ("s1", "b")))                 # s1 has two Emails -> uniqueness violation
    Dd, _ = forml.compile_model("It is obligatory that each Student has at most one Email.")
    with defs.step(Dd):                                       # constraint objects live in Dd's DEFS
        _p, v, flag = from_lam(apply(forml.validate_for("Student_has_Email", Dd), _pd(viol, Dd)))
    assert set(v) == {("s1", "a"), ("s1", "b")} and flag == "F"   # deontic -> reported, would commit
    Da, _ = forml.compile_model("Each Student has at most one Email.")
    with defs.step(Da):
        _p, v, flag = from_lam(apply(forml.validate_for("Student_has_Email", Da), _pd(viol, Da)))
    assert flag == "T"                                        # alethic -> would block commit
