"""Tromp diagrams + the rho-fidelity checker (engine tasks 20 & 22).

The load-bearing-math litmus, exercised end to end:
  (a) the reified pure-lambda reducer takes I / K / S / TRUE to their expected normal forms;
  (b) a populated fact lifts to  lambda f. f(o1..on) , renders a sane Tromp diagram, reduces to
      that normal form, and projects each object by application (membership IS application);
  (c) the CHECKER lifts ALL 353 canon DEFs and certifies each a CLOSED, WELL-FORMED pure term
      --- reporting honestly the ones that reach the registered host boundary.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import host_py.tromp as T
from host_py.tromp import (Var, Lam, App, Atom, Lm, _run, normalize, is_closed,
                           well_formed, fact_to_term, church_selector, render_ascii,
                           render_svg, CanonLift, check_canon, check_facts)


# --- reusable atoms for reduction tests ---
_a = Atom("a", label="a")
_b = Atom("b", label="b")
_c = Atom("c", label="c")


# ============================================================================
# (a) known combinators reduce to the expected normal forms
# ============================================================================
def _S():
    return _run(Lm(lambda f: Lm(lambda g: Lm(lambda x: f(x)(g(x))))))


def test_combinators_are_closed_and_well_formed():
    for b in (T.I, T.K, T.TRUE, T.FALSE, T.Y):
        term = _run(b)
        assert well_formed(term)
        assert is_closed(term)
    assert is_closed(_S()) and well_formed(_S())


def test_identity_reduces():
    I = _run(T.I)
    assert I == Lam(Var(0))                       # \x. x
    got, normal = normalize(App(I, _a))
    assert normal and got == _a


def test_K_selects_first():
    K = _run(T.K)
    assert K == Lam(Lam(Var(1)))                  # \x.\y. x
    got, normal = normalize(App(App(K, _a), _b))
    assert normal and got == _a


def test_church_booleans():
    TRUE, FALSE = _run(T.TRUE), _run(T.FALSE)
    assert normalize(App(App(TRUE, _a), _b))[0] == _a
    assert normalize(App(App(FALSE, _a), _b))[0] == _b
    # a boolean IS its selector: TRUE is exactly K
    assert TRUE == _run(T.K)


def test_S_K_K_is_identity():
    # the textbook fact  S K K = I  (extensionally), reached by full normal-order reduction
    S, K, I = _S(), _run(T.K), _run(T.I)
    got, normal = normalize(App(App(S, K), K))
    assert normal
    assert got == I                               # \x. x
    # and it acts as identity on a concrete atom
    assert normalize(App(App(App(S, K), K), _c))[0] == _c


def test_non_normalizing_term_returns_partial_not_hang():
    # Omega = (\x. x x)(\x. x x) has no normal form; the fuel bound must return, not hang
    omega = _run(Lm(lambda x: x(x))(Lm(lambda x: x(x))))
    _term, normal = normalize(omega, fuel=500)
    assert normal is False                        # flagged non-normalizing, no crash / no hang


# ============================================================================
# (b) a populated fact: lambda f. f(o1..on), Tromp diagram, and membership=application
# ============================================================================
def test_fact_shape_is_lambda_f_dot_f_objects():
    fact = fact_to_term(("Alice", "Bob"))
    # exactly  Lam(App(App(Var0, Atom Alice), Atom Bob))
    assert fact == Lam(App(App(Var(0), Atom("Alice", "Alice")), Atom("Bob", "Bob")))
    assert is_closed(fact) and well_formed(fact)


def test_fact_reduces_to_its_normal_form():
    for objs in [("Alice", "Bob"), ("Person", "has", "Name"), ("Widget",)]:
        fact = fact_to_term(objs)
        nf, normal = normalize(fact)
        assert normal
        assert nf == fact_to_term(objs)           # the rho-fidelity equality: nf == lambda f. f(o..)


def test_membership_is_application():
    # applying the fact to the i-th projection yields o_i --- a real beta reduction
    objs = ("Person", "has", "Name")
    fact = fact_to_term(objs)
    n = len(objs)
    for i, o in enumerate(objs, start=1):
        got, normal = normalize(App(fact, church_selector(i, n)))
        assert normal and got == Atom(o, label=o)


def test_fact_ascii_is_sane():
    art = render_ascii(fact_to_term(("Alice", "Bob"))).splitlines()
    assert any("Alice" in ln for ln in art)       # the objects appear as leaves
    assert any("Bob" in ln for ln in art)
    assert any(set(ln) <= set(" -+|") and "-" in ln for ln in art)   # a horizontal bar exists
    assert any("|" in ln for ln in art)                              # a variable/connector exists


def test_fact_svg_written(tmp_path):
    path = os.path.join(str(tmp_path), "fact.svg")
    svg = T.write_fact_svg(("Person", "has", "Name"), path)
    assert os.path.exists(path) and os.path.getsize(path) > 0
    assert svg.lstrip().startswith("<svg")
    assert "<line" in svg and "Person" in svg


def test_svg_and_ascii_share_one_layout():
    # both renderers consume the SAME layout, so the segment counts agree
    fact = fact_to_term(("Alice", "Bob"))
    lay = T.layout(fact)
    svg = render_svg(fact)
    assert svg.count("<line") == len(lay.hbars) + len(lay.vlines)


# ============================================================================
# (c) THE CHECKER over ALL 362 canon DEFs
# ============================================================================
# The honest finding, pinned so any drift is caught. 345 DEFs lift to closed pure terms; 17
# transitively reach five REGISTERED host string primitives (implode / escape_html / slug / lex
# / strip_prefix) --- the enumerable boundary (paper eq.(boundary): "the line at which
# Turing-complete computation re-enters"), not the pure applicative subsystem. (2026-07-14:
# +9 pure-closed for task 16 — the pop_of_spec absorbed-view family (4: pop_of_spec +
# subset/mandatory_facts/equality_side _spec) and the participation_spec exclusion family
# (5: part_one_spec, participation_spec, scoped_{exclusion,exclusive_or,inclusive_or}_spec),
# which retired the last host compositions; all nine lift to closed pure terms.)
_HOST_PRIMS = {"implode", "escape_html", "slug", "lex", "strip_prefix"}
_EXPECTED_BOUNDARY = {
    "system:ev_base", "system:ev_item", "system:ev_step", "system:ev_cols", "system:repr",
    "system:ev_colpairs", "system:ev_fields", "system:entity_view", "system:sqlname",
    "system:sqlcol_base", "system:sqlcol", "system:cf_drop_min", "system:cf_drop_full",
    "system:cs_cid", "system:cs_rows", "system:clause_ft", "system:render_html",
}


def test_every_canon_def_lifts_to_a_closed_pure_term():
    lift = CanonLift.load()
    rep = check_canon(lift)

    assert rep.total == 362
    # no UNEXPECTED malformation: every DEF is either pure-closed or a known boundary DEF
    assert rep.malformed == [], f"unexpected malformations: {rep.malformed}"

    # each certified DEF is genuinely a closed, well-formed pure lambda term
    for name in rep.pure_closed:
        term = lift.def_to_term(name)
        assert well_formed(term), name
        assert is_closed(term), name

    assert len(rep.pure_closed) == 345
    assert len(rep.boundary) == 17
    assert len(rep.pure_closed) + len(rep.boundary) == rep.total


def test_boundary_defs_are_exactly_the_registered_host_reachers():
    lift = CanonLift.load()
    rep = check_canon(lift)
    got = {name for name, _atoms in rep.boundary}
    assert got == _EXPECTED_BOUNDARY
    # every boundary DEF's lift is well-formed but NOT closed (it reaches a host primitive)...
    for name, atoms in rep.boundary:
        term = lift.def_to_term(name)
        assert well_formed(term) and not is_closed(term), name
        # ...and the offending atoms are exactly registered host primitives
        assert set(atoms) <= _HOST_PRIMS, (name, atoms)


def test_recursion_is_the_Y_combinator():
    # exactly the two self-recursive DEFs need Y to lift; they still lift to closed pure terms
    lift = CanonLift.load()
    rep = check_canon(lift)
    assert set(rep.recursive) == {"system:ins_desc", "system:ins_asc"}
    for name in rep.recursive:
        term = lift.def_to_term(name)
        assert is_closed(term) and well_formed(term)


def test_facts_certify_rho_fidelity():
    rep = check_facts()
    assert rep.fact_checks and all(ok for _objs, ok in rep.fact_checks)


def test_a_leaf_def_lifts_to_a_small_self_contained_term():
    # theta:append_phi = COMP(apndr, CONS(id, CONST(PHI))) --- no cross-refs, so a compact term
    lift = CanonLift.load()
    term = lift.def_to_term("theta:append_phi")
    assert is_closed(term) and well_formed(term)
    assert T.size(term) < 40                       # genuinely small (no environment plumbing)
