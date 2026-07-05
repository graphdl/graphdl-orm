"""The builders migration (the thin-runner endgame, per the Operating Rule
defs-override-glue-framework): each Python tree-builder in system.py moves to
a shared canonical DEF, proven by the twin oracle. The Python builder is the
behavioral specification; the canonical def applied to the same operands must
answer the same rows. Hosts then need only the reducer and the shared file,
and a host-specific builder becomes an isomorphic DEFS override, never the
meaning."""
import pyarest.prims  # noqa: F401
from pyarest.lam import atom as A, from_lam, to_lam
from pyarest.reduce import apply as _ap


SM_POPS = (
    (("t1", "draft"), ("t2", "review")),          # smFrom ⟨t, from⟩
    (("t1", "submit"), ("t2", "approve")),        # smTrigger ⟨t, trigger⟩
    (("t1", "review"), ("t2", "done")),           # smTo ⟨t, to⟩
)


def test_sm_join_canonical_def_derives_the_machine_triples():
    # the Python builder was the twin oracle during migration; the behavioral
    # pin is what stays (the def IS the implementation now, in every host)
    pops = to_lam(SM_POPS)
    via_def = from_lam(_ap(A("system:sm_join"), pops))
    assert set(via_def) == {("draft", "submit", "review"),
                            ("review", "approve", "done")}


def test_sm_join_named_canonical_def_keys_by_transition():
    pops = to_lam(SM_POPS)
    via_def = from_lam(_ap(A("system:sm_join_named"), pops))
    assert set(via_def) == {("t1", "draft", "submit", "review"),
                            ("t2", "review", "approve", "done")}


LINKS = (("a", "b"), ("b", "c"), ("c", "d"))


def test_join_rule_canonical_builder_matches_the_python_builder():
    # ancestor(x,z) <- link(x,y), ancestor(y,z): join role 2, head [1, 3]
    from pyarest import system
    args = to_lam((2, (1, 3)))
    via_def = _ap(_ap(A("system:join_rule"), args), to_lam(LINKS))
    via_python = _ap(system.join_rule(2, [1, 3]), to_lam(LINKS))
    assert set(from_lam(via_def)) == set(from_lam(via_python))
    assert set(from_lam(via_def)) == {("a", "c"), ("b", "d")}


def test_join_rule2_canonical_builder_matches_the_python_builder():
    # FastCarDriver(x) <- drives(x,y), isFast(y): two cells, join role 2
    from pyarest import system
    drives = (("ann", "car1"), ("bob", "car2"))
    fast = (("car2",),)
    args = to_lam((2, (1,)))
    pops = to_lam((drives, fast))
    via_def = _ap(_ap(A("system:join_rule2"), args), pops)
    via_python = _ap(system.join_rule2(2, [1]), pops)
    assert set(from_lam(via_def)) == set(from_lam(via_python))
    assert set(from_lam(via_def)) == {("bob",)}


def test_derive_of_canonical_builder_computes_the_closure():
    # the recursive head resolved by the least fixed point: transitive closure
    # of LINKS through the canonical F_of/derive_of, against the Python twins
    from pyarest import system
    rule_args = to_lam((2, (1, 3)))
    rule = _ap(A("system:join_rule"), rule_args)
    via_def = _ap(_ap(A("system:derive_of"), to_lam(())), to_lam(LINKS))
    # empty rules: derive is the identity (matches the Python guard)
    assert set(from_lam(via_def)) == set(LINKS)
    closure_def = _ap(_ap(A("system:derive_of"), lam_seq([rule])),
                      to_lam(LINKS))
    closure_py = _ap(system.derive_of([system.join_rule(2, [1, 3])]),
                     to_lam(LINKS))
    assert set(from_lam(closure_def)) == set(from_lam(closure_py))
    assert ("a", "d") in set(from_lam(closure_def))


def lam_seq(objs):
    # a SEQUENCE of already-reduced rule objects (to_lam would try to encode
    # them as data; the S constructor keeps them as the sequence's elements)
    from pyarest.lam import SEQ, NIL, CONS
    out = NIL
    for o in reversed(objs):
        out = CONS(o)(out)
    return SEQ(out)


def test_mint_next_canonical_builder_matches_the_python_builder():
    from pyarest import system
    pop = to_lam((("2", "x"), ("7", "y"), ("4", "z")))
    empty = to_lam(())
    via_def = _ap(_ap(A("system:mint_next"), A(1)), pop)
    via_python = _ap(system.mint_next(1), pop)
    assert from_lam(via_def) == from_lam(via_python)
    assert from_lam(via_def) == 8
    assert from_lam(_ap(_ap(A("system:mint_next"), A(1)), empty)) == 1


def test_resolve_minting_canonical_builder_matches_the_python_builder():
    from pyarest import system
    arg = to_lam((("alice",), (("1", "bob"), ("3", "cara"))))
    via_def = _ap(_ap(A("system:resolve_minting"), A(1)), arg)
    via_python = _ap(system.resolve_minting(1), arg)
    assert from_lam(via_def) == from_lam(via_python)
    got = from_lam(via_def)
    # the minted fact ⟨4, alice⟩ prepends to the population
    assert got[0] == (4, "alice") and len(got) == 3


RING_POP = (("a", "a"), ("a", "b"), ("b", "a"), ("b", "c"))


def test_ring_irreflexive_canonical_matches_the_python_builder():
    from pyarest import system
    via_def = _ap(_ap(A("constraints:ring_irreflexive"), to_lam((1, 2))),
                  to_lam(RING_POP))
    via_python = _ap(system.ring_irreflexive((1, 2)), to_lam(RING_POP))
    assert set(from_lam(via_def)) == set(from_lam(via_python))
    assert set(from_lam(via_def)) == {("a", "a")}


def test_ring_symmetric_canonical_matches_the_python_builder():
    from pyarest import system
    via_def = _ap(_ap(A("constraints:ring_symmetric"), to_lam((1, 2))),
                  to_lam(RING_POP))
    via_python = _ap(system.ring_symmetric((1, 2)), to_lam(RING_POP))
    assert set(from_lam(via_def)) == set(from_lam(via_python))
    # (b,c) lacks its reverse; (a,b)/(b,a) satisfy each other; (a,a) is its own
    assert set(from_lam(via_def)) == {("b", "c")}


def test_ring_asymmetric_canonical_matches_the_python_builder():
    from pyarest import system
    via_def = _ap(_ap(A("constraints:ring_asymmetric"), to_lam((1, 2))),
                  to_lam(RING_POP))
    via_python = _ap(system.ring_asymmetric((1, 2)), to_lam(RING_POP))
    assert set(from_lam(via_def)) == set(from_lam(via_python))
    # (a,a) swaps to itself; (a,b)/(b,a) swap to each other: all three violate
    assert set(from_lam(via_def)) == {("a", "a"), ("a", "b"), ("b", "a")}


def test_ring_antisymmetric_canonical_matches_the_python_builder():
    from pyarest import system
    via_def = _ap(_ap(A("constraints:ring_antisymmetric"), to_lam((1, 2))),
                  to_lam(RING_POP))
    via_python = _ap(system.ring_antisymmetric((1, 2)), to_lam(RING_POP))
    assert set(from_lam(via_def)) == set(from_lam(via_python))
    # reflexive (a,a) allowed; the (a,b)/(b,a) pair violates both ways
    assert set(from_lam(via_def)) == {("a", "b"), ("b", "a")}


def test_ring_intransitive_canonical_matches_the_python_builder():
    from pyarest import system
    pop = (("a", "b"), ("b", "c"), ("a", "c"), ("c", "d"))
    via_def = _ap(_ap(A("constraints:ring_intransitive"), to_lam((1, 2))),
                  to_lam(pop))
    via_python = _ap(system.ring_intransitive((1, 2)), to_lam(pop))
    assert set(from_lam(via_def)) == set(from_lam(via_python))
    assert set(from_lam(via_def)) == {("a", "c")}


def test_ring_acyclic_canonical_matches_the_python_builder():
    from pyarest import system
    pop = (("a", "b"), ("b", "c"), ("c", "a"), ("x", "y"))
    via_def = _ap(_ap(A("constraints:ring_acyclic"), to_lam((1, 2))),
                  to_lam(pop))
    via_python = _ap(system.ring_acyclic((1, 2)), to_lam(pop))
    assert set(from_lam(via_def)) == set(from_lam(via_python))
    assert set(from_lam(via_def)) == {("a", "a"), ("b", "b"), ("c", "c")}


def test_class_rule_canonical_matches_the_python_builder():
    # the grammar recognizer as one FFP object: clauses intersect statement
    # ids by field value (or existence), the head pairs survivors with the
    # classification constant
    from pyarest import system, ast
    from pyarest.reduce import apply as _apply
    D = _apply(ast.Store("Statement_has_Keyword"),
               _S(to_lam((("s1", "iff"), ("s2", "iff"), ("s3", "if"))),
                  _apply(ast.Store("Statement_has_Verb"),
                         _S(to_lam((("s1", "has"), ("s3", "has"))),
                            to_lam(())))))
    clauses = (("Statement_has_Keyword", "iff"), ("Statement_has_Verb", None))
    via_python = _apply(system.class_rule(list(clauses), "Derivation Rule"), D)
    pred = _S(A("COMP"), A("eq"),
              _S(A("CONS"), A(2), _S(A("CONST"), A("iff"))))
    cl = _S(_S(A("Statement_has_Keyword"), pred),
            _S(A("Statement_has_Verb"), to_lam(())))
    via_def = _apply(_apply(A("system:class_rule"),
                            _S(cl, A("Derivation Rule"))), D)
    assert set(from_lam(via_def)) == set(from_lam(via_python))
    # s1 has both; s2 lacks the verb; s3 has the wrong keyword
    assert set(from_lam(via_def)) == {("s1", "Derivation Rule")}


def _S(*xs):
    import pyarest.lam as L
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def test_ftpop_absorbed_canonical_matches_the_python_builder():
    # the absorbed fact type's population reassembled through the index and
    # the dynamic fetch (the create pipeline's layout: the entity cell holds
    # the flat 3NF row, holes as '#'); a holed column drops outer-join style
    from pyarest import system, ast
    from pyarest.reduce import apply as _apply
    partition = {"Person_has_Name": "Person", "Person": "Person"}
    D = _apply(ast.Store("Person"),
               _S(to_lam((("p1",), ("p2",), ("p3",))),
                  _apply(ast.Store("Person:p1"),
                         _S(to_lam(("p1", "Ada")),
                            _apply(ast.Store("Person:p2"),
                                   _S(to_lam(("p2", "Bo")),
                                      _apply(ast.Store("Person:p3"),
                                             _S(to_lam(("p3", "#")),
                                                to_lam(())))))))))
    via_python = _apply(system.ftpop_expr("Person_has_Name", partition), D)
    got = set(from_lam(via_python))
    assert got == {("p1", "Ada"), ("p2", "Bo")}, got
    col = 2 + system.table_columns(partition, "Person").index("Person_has_Name")
    via_def = _apply(_apply(A("system:ftpop_absorbed"),
                            _S(A("Person"), A(col))), D)
    assert set(from_lam(via_def)) == got


def test_row_validate_canonical_matches_the_python_builder():
    # the routed write's value check: the row's column against the named vc,
    # holes skipped, the flag alethic per modality
    from pyarest import system, defs
    from pyarest.reduce import apply as _apply
    from pyarest import canon as T
    defs.define("Mood_vc", T.Filter(_S(A("COMP"), A("not"), A("eq"),
                                       _S(A("CONS"), A(1),
                                          _S(A("CONST"), A("calm"))))))
    row_ok = to_lam(("t1", "calm"))
    row_bad = to_lam(("t1", "wild"))
    row_hole = to_lam(("t1", "#"))
    via_def = _apply(A("system:row_validate"),
                     _S(A(2), A("Mood_vc"), A("T")))
    for row, want_v, want_flag in ((row_ok, (), "F"),
                                   (row_bad, (("wild",),), "T"),
                                   (row_hole, (), "F")):
        got = from_lam(_apply(via_def, _S(row, to_lam(()))))
        assert got[1] == want_v and got[2] == want_flag, (got, want_v, want_flag)


def test_verbalize_pairs_canonical_over_the_entity_facts():
    # synthesize's engine half (the old verb's contract: the engine
    # guarantees content, the LLM only shapes wording): the entity's facts
    # paired with their fact types' reading templates, post-derive, as
    # structured pairs; rendering is a connector concern
    from pyarest import forml, system
    from pyarest.reduce import apply as _apply
    model = """Person(.nr) is an entity type.
Name is a value type.
Pet is an entity type.
Person has Name.
Person keeps Pet.
Person 'p1' has Name 'Ada'.
Person 'p1' keeps Pet 'rex'.
Person 'p2' has Name 'Bo'.
"""
    D, rep = forml.compile_model(model)
    D = system.run_rules(D)
    got = from_lam(_apply(_apply(A("system:verbalize"), A("p1")), D))
    pairs = {(r[0], tuple(r[1])) for r in got}
    assert ("{0} has {1}", ("p1", "Ada")) in pairs
    assert ("{0} keeps {1}", ("p1", "rex")) in pairs
    assert not any("Bo" in str(p) for p in pairs)


def test_iota_generates_the_selector_sequence():
    # the missing tool for width-parameterized builders (row_resolve,
    # checked_apply): n to ⟨1..n⟩, the WHILE countdown growing the sequence
    got = from_lam(_ap(A("theta:iota"), A(4)))
    assert got == (1, 2, 3, 4)
    assert from_lam(_ap(A("theta:iota"), A(1))) == (1,)
    assert from_lam(_ap(A("theta:iota"), A(0))) == ()


def test_row_resolve_canonical_matches_the_python_builder():
    # the entity-cell write resolver: fresh rows hole-padded, compatible
    # updates land, conflicting functional writes collapse the row (the UC
    # made structural); the quasiquote pattern builds the per-column
    # expressions from the width through theta:iota
    from pyarest import system
    op = _ap(_ap(A("system:row_resolve"), _S(A(3), A(4), A("F"))),
             _S(_S(A("k1"), A("v1")), to_lam(())))
    assert from_lam(op) == ("k1", "#", "v1", "#")
    py = system.row_resolve(3, 4)
    canon = _ap(A("system:row_resolve"), _S(A(3), A(4), A("F")))
    for case in (_S(_S(A("k1"), A("v1")), to_lam(("k1", "a", "#", "b"))),
                 _S(_S(A("k1"), A("v1")), to_lam(("k1", "a", "OTHER", "b")))):
        assert from_lam(_ap(canon, case)) == from_lam(_ap(py, case))


def test_checked_apply_canonical_matches_the_python_builder():
    # the typed boundary (Def. reg dom/cod), the last pipeline mover: apply
    # iff every declared dom position holds an instance of its type, else the
    # ERROR atom the transition rule refuses
    from pyarest import system, ast, defs
    from pyarest.reduce import apply as _apply
    defs.define("double", _S(A("COMP"), A("+"),
                             _S(A("CONS"), A(1), A(1))))
    D = _apply(ast.Store("defSig"),
               _S(to_lam((("double", 1, "Num"),)),
                  _apply(ast.Store("Num"),
                         _S(to_lam((("3",), ("4",))), to_lam(())))))
    py = system.checked_apply("double")
    canon = _apply(A("system:checked_apply"), A("double"))
    for case in (_S(to_lam(("3",)), D), _S(to_lam(("9",)), D)):
        assert from_lam(_apply(canon, case)) == from_lam(_apply(py, case))
    assert from_lam(_apply(canon, _S(to_lam(("9",)), D))) == "ERROR"
