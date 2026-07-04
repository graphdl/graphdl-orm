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
