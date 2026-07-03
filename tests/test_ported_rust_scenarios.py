"""Scenarios extracted from the old Rust engine's semantic test suite, run against the
substrate (per the polyglot insight, they are DATA — readings, operations, expected
results — not Python ports; the kernels already agree, so one execution judges the old
expectation against the design). Verdicts per docs/2026-07-02-rust-test-triage.md:

VALID (the old contract holds here):
  ring_constraint_enforcement_e2e — irreflexive/asymmetric fire on offenders, alethic
  hard-reject, zero false positives on a clean graph.
  recursive_self_join_closure_e2e — transitive closure to the lfp (surface adapted to
  the book's numbered-variable rule form; same semantics).

VALID SPEC, was a gap in BOTH engines, now CLOSED here (the clause lift):
  subtype_join_antecedent_supertype_ft_e2e — a subtype-keyed clause must resolve up to
  the supertype-declared fact type (subtype instances ARE supertype instances).

SUPERSEDED (the old semantics diverge from the sources; the adapted form passes):
  unary_derivation_e2e — 'X is not Y' in rule BODIES with 2-stratum negation chaining.
  Rule bodies are positive by construction (the whitepaper's groundedness paragraph;
  NORMA refuses negated actions); negative information is the PAIRED fact type
  (UnaryValuePattern), referenced positively. The adapted scenario derives through
  the pair."""
import pytest
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


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


RING = """Task(.id) is an entity type.
Task blocks Task.
Task blocks Task is irreflexive.
Task blocks Task is asymmetric.
Task 't1' blocks Task 't1'.
Task 't2' blocks Task 't3'.
Task 't3' blocks Task 't2'.
"""


def test_ring_constraints_fire_on_self_block_and_reciprocal_pair():
    D, rep = forml.compile_model(RING)
    assert rep["unparsed"] == []
    vo = forml.validate_for("Task_blocks_Task", D)
    pop = apply(ast.FetchPop("Task_blocks_Task"), D)
    with defs.step(D):
        (_p, V, flag) = from_lam(apply(vo, S(pop, D)))
    offenders = set(V)
    assert ("t1", "t1") in offenders                          # irreflexive fires
    assert ("t2", "t3") in offenders or ("t3", "t2") in offenders   # asymmetric fires
    assert flag == "T"                                        # alethic: hard-reject


def test_ring_violating_write_never_lands():
    # contract leg 3: create_via_defs hard-rejects (D' = D)
    D, _ = forml.compile_model("""Task(.id) is an entity type.
Task blocks Task.
Task blocks Task is irreflexive.
""")
    vo = forml.validate_for("Task_blocks_Task", D)
    Dp = apply(A(2), ast.run(to_lam(("t1", "t1")), D, validate_obj=vo,
                             cell_name="Task_blocks_Task"))
    assert ("t1", "t1") not in _cell(from_lam(Dp), "Task_blocks_Task")


def test_ring_constraints_pass_on_clean_block_graph():
    CLEAN = """Task(.id) is an entity type.
Task blocks Task.
Task blocks Task is irreflexive.
Task blocks Task is asymmetric.
Task 't1' blocks Task 't2'.
Task 't2' blocks Task 't3'.
Task 't1' blocks Task 't3'.
"""
    D, _ = forml.compile_model(CLEAN)
    vo = forml.validate_for("Task_blocks_Task", D)
    pop = apply(ast.FetchPop("Task_blocks_Task"), D)
    with defs.step(D):
        (_p, V, _f) = from_lam(apply(vo, S(pop, D)))
    assert set(V) == set()                                    # zero false positives


def test_recursive_self_join_closure_reaches_the_lfp():
    # recursive_self_join_closure_e2e, the book's rule surface
    MODEL = """Glyph is an entity type.
Glyph links Glyph.
Glyph1 reaches Glyph2 if Glyph1 links Glyph2.
Glyph1 reaches Glyph3 if Glyph1 links Glyph2 and Glyph2 reaches Glyph3.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    D = apply(ast.Store("Glyph_links_Glyph"), S(to_lam((("a", "b"), ("b", "c"), ("c", "d"))), D))
    D = system.run_rules(D)
    got = _cell(from_lam(D), "Glyph_reaches_Glyph")
    assert got == {("a", "b"), ("b", "c"), ("c", "d"), ("a", "c"), ("b", "d"), ("a", "d")}


def test_subtype_clause_resolves_up_to_the_supertype_fact_type():
    # the gap BOTH engines shared, now closed here: the clause lift resolves a
    # subtype-keyed atom up to the supertype-declared fact type
    MODEL = """Function(.id) is an entity type.
Domain(.id) is an entity type.
Resource(.id) is an entity type.
Noun is an entity type.
Noun is a subtype of Function.
Resource is instance of Noun.
Function belongs to Domain.
Resource1 belongs to Domain2 if Resource1 is instance of Noun1 and Noun1 belongs to Domain2.
"""
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.Store("Resource_is_instance_of_Noun"), S(to_lam((("r1", "n1"),)), D))
    D = apply(ast.Store("Function_belongs_to_Domain"), S(to_lam((("n1", "d1"),)), D))
    D = system.run_rules(D)
    assert ("r1", "d1") in _cell(from_lam(D), "Resource_belongs_to_Domain")


def test_unary_negation_in_rule_bodies_is_superseded_by_the_pair():
    # unary_derivation_e2e's 'Task is parallelizable iff … and Task is not
    # file-conflicting' — adapted: the negation is the PAIRED fact type, referenced
    # positively; no stratification machinery exists or is needed
    MODEL = """Task(.id) is an entity type.
Task is file-conflicting.
Task is not file-conflicting.
Task is ready.
Task1 is parallelizable if Task1 is ready and Task1 is not file-conflicting.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    Dpy = from_lam(D)
    assert ("Task_is_not_file-conflicting", "Task_is_file-conflicting") in _cell(Dpy, "negOf") \
        or ("Task_is_not_file_conflicting", "Task_is_file_conflicting") in _cell(Dpy, "negOf")
    D = apply(ast.Store("Task_is_ready"), S(to_lam((("t1",), ("t2",))), D))
    neg_ft = next(iter(r[0] for r in _cell(Dpy, "negOf")))
    D = apply(ast.Store(neg_ft), S(to_lam((("t1",),)), D))    # t1 asserted NOT conflicting
    D = system.run_rules(D)
    derived = {r[0] for r in _cell(from_lam(D), "Task_is_parallelizable")}
    assert "t1" in derived                                    # derives through the pair
    assert "t2" not in derived                                # unknown stays unknown (open world)


def test_literal_comparators_filter_in_rule_bodies():
    # antecedent_literal_value_comparison_e2e, the corpus word-comparator surface:
    # a clause whose subject is a BOUND variable and whose object is a literal is a
    # FILTER over the running tuple, not a join atom
    MODEL = """Item(.Id) is an entity type.
Weight is a value type.
Item has Weight.
Item1 is big if Item1 has Weight2 and Weight2 is greater than 5.
Item1 is small if Item1 has Weight2 and Weight2 is less than 5.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    D = apply(ast.Store("Item_has_Weight"),
              S(to_lam((("a", 3), ("b", 7), ("c", 5))), D))
    D = system.run_rules(D)
    Dpy = from_lam(D)
    assert {r[0] for r in _cell(Dpy, "Item_is_big")} == {"b"}
    assert {r[0] for r in _cell(Dpy, "Item_is_small")} == {"a"}   # c is neither
