"""#18: the rule-form reading translators (rule_if / rule_iff), canonized.
_cook_rule_if resolves the ENTIRE body parse at the Stage-1 boundary (clause
split, column map, comparators-as-filter-trees, coercion aliases, negation
groups, the aggregate extraction, the head shape incl. skolem existentials);
the translator is the generic system:h_crows body (hosts _h_rule_if/_h_rule_iff
are the crows aliases), and every compiled object is an obj SPEC applied
through DEFS — system:compile_rule (+ one compile_rule_delta per atom seat),
system:compile_agg_rule, and the NEW system:compile_rule_neg (the stratified
anti-join fold engine.compile_rule_neg hosts, certified here as its twin).
Filters ride the specs as the cmp_filter DATA TREES, so the apply prim rebuilds
the IDENTICAL objects the engine builders construct — certified STRUCTURALLY
(from_lam equality against the engine builders with the old monolith's exact
arguments) and EXTENSIONALLY (derivations over sample cells, non-vacuous)."""
import zlib

import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest import canon, compiler, defs, system
from pyarest.compiler import _Known, _cook_rule_if, _cook_rule_iff
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

canon.load_all()

K = _Known({"Person", "Car", "Age", "Task", "Cost", "Tally", "Layer",
            "Stratum Stack", "Engineering Lever", "Load", "View",
            "View Element", "Transition"}, {}, set(), set())

# the six head shapes, raw ⟨head, body⟩ as Stage-1's rule_if production captures them
PLAIN = ("Person1 is risky", "Person1 smokes")
JOIN2 = ("Person1 is risky", "Person1 owns Car1 and Car1 is fast")
FILTERED = ("Person1 is senior", "Person1 has Age1 and Age1 is at least 65")
BODYLIT = ("Layer1 is grounded", "Layer1 has Load '0'")
NEGATION = ("Layer1 has Load '0'",
            "Layer1 stacks into Stratum Stack1 and no Engineering Lever loads Layer1")
AGGREGATE = ("Task1 has Tally", "Tally is the count of Cost1 where Task1 has Cost1")
SKOLEM = ("View Element1 renders Transition1", "View1 offers Transition1")


def _cid(head, body):
    hft, _v, _l = compiler._rule_atom(head, K)
    return hft + "_rule_" + format(zlib.crc32(body.encode()), "x")


def _canon(groups, name="system:h_rule_if"):
    r = R(A(name), to_lam((groups, (), "alethic")))
    asserts = [(x[0], tuple(x[1])) for x in from_lam(R(A(1), r))]
    objs = from_lam(R(A(2), r))                     # ⟨⟨cid, obj-tree⟩…⟩, whole
    return asserts, [(p[0], p[1]) for p in objs]


def _host(groups):
    a, o = compiler._h_rule_if(groups, None, "alethic")
    return [(c, tuple(r)) for c, r in a], [(cid, from_lam(obj)) for cid, obj in o]


def _D(*cells):
    return to_lam(tuple(("CELL", n, v) for (n, v) in cells))


def _derive(obj_tree, D):
    with defs.step(D):
        return set(map(tuple, from_lam(R(to_lam(obj_tree), D)) or ()))


# ---- the cook: golden spec shapes (the old monolith's builder arguments, as data) ----

def test_cook_plain_copy_rule_specs_and_rows():
    rows, mid, ospecs = _cook_rule_if(PLAIN, K)
    cid = _cid(*PLAIN)
    assert mid == ()
    assert ospecs == (
        (cid, "system:compile_rule", ((("Person_smokes", 1, ()),), (1,), ())),
        (cid + "~d1", "system:compile_rule_delta",
         ((("Person_smokes", 1, ()),), (1,), (), 1)))
    assert rows[0] == ("factType", ("Person_is_risky", "{0} is risky"))
    assert ("derivation", ("Person_is_risky", "fully-derived")) in rows
    assert ("ruleDerives", (cid, "Person_is_risky")) in rows
    assert ("ruleReads", (cid, "Person_smokes")) in rows
    assert ("derivationRule", ("Person_is_risky", "Person_smokes", 1)) in rows
    assert ("ruleCopies", (cid, "Person_smokes", "Person_is_risky")) in rows
    assert ("ruleAtom", (cid, 1, "Person_smokes")) in rows


def test_cook_join_linear_chain_and_delta_seats():
    _rows, _mid, ospecs = _cook_rule_if(JOIN2, K)
    atoms = (("Person_owns_Car", 2, ()), ("Car_is_fast", 1, ()))
    assert ospecs[0][1:] == ("system:compile_rule", (atoms, (1,), ()))
    assert [o[1] for o in ospecs[1:]] == ["system:compile_rule_delta"] * 2
    assert [o[2][3] for o in ospecs[1:]] == [1, 2]  # one ~d per atom seat, 1-based


def test_cook_comparator_and_body_literal_are_filter_trees():
    _r, _m, ospecs = _cook_rule_if(FILTERED, K)
    assert ospecs[0][2] == ((("Person_has_Age", 2, ()),), (1,),
                            (("COMP", "ge", ("CONS", 2, ("CONST", 65))),))
    _r, _m, ospecs = _cook_rule_if(BODYLIT, K)
    assert ospecs[0][2] == ((("Layer_has_Load", 2, ()),), (1,),
                            (("COMP", "eq", ("CONS", 2, ("CONST", 0))),))


def test_cook_negation_spec_and_rows():
    rows, _m, ospecs = _cook_rule_if(NEGATION, K)
    cid = _cid(*NEGATION)
    assert ospecs == ((cid, "system:compile_rule_neg", (
        (("Layer_stacks_into_Stratum_Stack", 2, ()),),
        (1, ("CONST", 0)),                          # head: bound col + literal
        (1, 2),                                     # identity body head 1..ncols
        (),
        (((("Engineering_Lever_loads_Layer", 2, ()),), (2,), (),
          ((1,), (1,))),))),)                       # ⟨anti_key, 1..|nproj|⟩
    assert ("ruleNeg", (cid,)) in rows
    assert not any(r[0] == "ruleAtom" for r in rows)  # no ~d under negation


def test_cook_aggregate_spec_and_rows():
    rows, _m, ospecs = _cook_rule_if(AGGREGATE, K)
    cid = _cid(*AGGREGATE)
    assert ospecs == ((cid, "system:compile_agg_rule",
                       ((("Task_has_Cost", 2, ()),), (1,), 2, "count", ())),)
    assert ("ruleAgg", (cid,)) in rows
    assert not any(r[0] == "ruleAtom" for r in rows)  # no ~d under aggregation


def test_cook_skolem_head_spec_and_rows():
    rows, _m, ospecs = _cook_rule_if(SKOLEM, K)
    cid = _cid(*SKOLEM)
    assert ospecs[0][2] == ((("View_offers_Transition", 2, ()),),
                            (("COMP", "skolem",
                              ("CONS", ("CONST", "View Element1"), 1, 2)), 2),
                            ())
    assert ("ruleSkolem", (cid, "View_Element_renders_Transition")) in rows


def test_cook_rule_iff_marker_resolves_the_kind():
    rows, _m, _o = _cook_rule_iff(("**",) + PLAIN, K)
    assert ("derivation", ("Person_is_risky", "derived-and-stored")) in rows
    rows, _m, _o = _cook_rule_iff((None,) + PLAIN, K)
    assert ("derivation", ("Person_is_risky", "fully-derived")) in rows


def test_cook_diag_rule_stays_m_facts_only():
    # an unbound head variable WITH a negation group: no skolem rescue (that
    # branch requires `not negs`) — the rule stays M-facts only and SAYS WHY
    rows, _m, ospecs = _cook_rule_if(
        ("Person1 is risky", "Task1 has Cost1 and no Car1 carries Task1"), K)
    assert ospecs == ()
    assert any(r[0] == "ruleDiag" for r in rows)


# ---- the translator twin: canon system:h_rule_if == host (rows AND objects) ----

def test_rule_translator_twins_host_canon():
    for raw in (PLAIN, JOIN2, FILTERED, BODYLIT, NEGATION, AGGREGATE, SKOLEM):
        g = _cook_rule_if(raw, K)
        ca, cobjs = _canon(g)
        ha, hobjs = _host(g)
        assert ca == ha, raw
        assert cobjs == hobjs, raw                  # STRUCTURAL object equality


def test_rule_iff_alias_twins_host():
    g = _cook_rule_iff(("*",) + JOIN2, K)
    ca, cobjs = _canon(g, name="system:h_rule_iff")
    ha, hobjs = _host(g)
    assert (ca, cobjs) == (ha, hobjs)


# ---- object fidelity: the specs rebuild the ENGINE BUILDERS' objects verbatim ----
# (the old monolith called these builders with exactly these arguments)

def _crows_objs(g):
    return compiler._h_crows(g, None, "alethic")[1]


def test_specs_rebuild_the_engine_builder_objects():
    want = {
        PLAIN: system.compile_rule(["Person_smokes"], [1], [1], [], []),
        JOIN2: system.compile_rule(["Person_owns_Car", "Car_is_fast"], [1],
                                   [2, 1], [], [None]),
        FILTERED: system.compile_rule(["Person_has_Age"], [1], [2],
                                      [system.cmp_filter("ge", 2, lit=65)], []),
        NEGATION: system.compile_rule_neg(
            ["Layer_stacks_into_Stratum_Stack"], [1, ("CONST", 0)], 2, [2],
            [], [], [(["Engineering_Lever_loads_Layer"], [2], [2], [], [], [1])]),
        AGGREGATE: system.compile_agg_rule(["Task_has_Cost"], [1], 2, "count",
                                           [2], [], []),
        SKOLEM: system.compile_rule(
            ["View_offers_Transition"],
            [("COMP", "skolem", ("CONS", ("CONST", "View Element1"), 1, 2)), 2],
            [2], [], []),
    }
    for raw, host_obj in want.items():
        built = _crows_objs(_cook_rule_if(raw, K))[0][1]
        assert from_lam(built) == from_lam(host_obj), raw


def test_delta_specs_rebuild_the_delta_builders():
    g = _cook_rule_if(JOIN2, K)
    objs = _crows_objs(g)
    for i in (0, 1):
        host_obj = system.compile_rule_delta(["Person_owns_Car", "Car_is_fast"],
                                             [1], i, [2, 1], [], [None])
        assert from_lam(objs[1 + i][1]) == from_lam(host_obj), i


# ---- extensional: the spec-built objects derive the right populations ----

def test_join_filter_agg_neg_extensional():
    cases = (
        (JOIN2, _D(("Person_owns_Car", (("p1", "c1"), ("p2", "c2"))),
                   ("Car_is_fast", (("c1",),))), {("p1",)}),
        (FILTERED, _D(("Person_has_Age", (("p1", 70), ("p2", 30)))), {("p1",)}),
        (BODYLIT, _D(("Layer_has_Load", (("L1", 0), ("L2", 5)))), {("L1",)}),
        (NEGATION, _D(("Layer_stacks_into_Stratum_Stack",
                       (("L1", "s"), ("L2", "s"))),
                      ("Engineering_Lever_loads_Layer", (("e1", "L2"),))),
         {("L1", 0)}),
        (AGGREGATE, _D(("Task_has_Cost", (("t1", 5), ("t1", 7), ("t2", 9)))),
         {("t1", 2), ("t2", 1)}),
    )
    for raw, D, want in cases:
        g = _cook_rule_if(raw, K)
        c_obj = _canon(g)[1][0][1]
        h_obj = _host(g)[1][0][1]
        assert c_obj == h_obj, raw
        got = _derive(c_obj, D)
        assert got == want, (raw, got)
        assert got, "non-vacuous"


def test_skolem_extensional_mints_the_same_ids():
    g = _cook_rule_if(SKOLEM, K)
    D = _D(("View_offers_Transition", (("v1", "t1"), ("v1", "t2"))))
    c_rows = _derive(_canon(g)[1][0][1], D)
    h_rows = _derive(_host(g)[1][0][1], D)
    assert c_rows == h_rows
    assert {r[1] for r in c_rows} == {"t1", "t2"}
    assert len({r[0] for r in c_rows}) == 2, "one fresh id per frontier binding"


# ---- system:compile_rule_neg: the new canon builder twins the host fold ----

def test_compile_rule_neg_canon_twins_host_structurally():
    operand = ((("R", 2, ()),), (1,), (1, 2), (),
               (((("Sx", 2, ()),), (1,), (), ((1,), (1,))),
                ((("Tx", 1, ()),), (1,), (), ((2,), (1,)))))
    with defs.step(L.SEQ(L.NIL)):
        built = R(A("system:compile_rule_neg"), to_lam(operand))
    host = system.compile_rule_neg(
        ["R"], [1], 2, [2], [], [],
        [(["Sx"], [1], [2], [], [], [1]),           # group 1 wraps first
         (["Tx"], [1], [1], [], [], [2])])          # group 2 wraps outermost
    assert from_lam(built) == from_lam(host)


def test_compile_rule_neg_canon_extensional_two_groups():
    operand = ((("R", 2, ()),), (1,), (1, 2), (),
               (((("Sx", 2, ()),), (1,), (), ((1,), (1,))),
                ((("Tx", 1, ()),), (1,), (), ((2,), (1,)))))
    D = _D(("R", (("a", 1), ("b", 2), ("c", 3))),
           ("Sx", (("a", 9),)),                     # anti col 1: drops ("a", 1)
           ("Tx", ((3,),)))                         # anti col 2: drops ("c", 3)
    with defs.step(L.SEQ(L.NIL)):
        built = R(A("system:compile_rule_neg"), to_lam(operand))
    host = system.compile_rule_neg(
        ["R"], [1], 2, [2], [], [],
        [(["Sx"], [1], [2], [], [], [1]), (["Tx"], [1], [1], [], [], [2])])
    with defs.step(D):
        got = set(map(tuple, from_lam(R(built, D))))
        want = set(map(tuple, from_lam(R(host, D))))
    assert got == want == {("b",)}
