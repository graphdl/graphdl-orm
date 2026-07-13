"""The induce ORACLE pins (canonize abduce/induce, slice a): the coin
fixture holds protocol.induce's answer shape so the canon reference
certifies against the same functions the verb runs. induce_domain is
module-level for exactly this reason: the coming canon system:role_domain
differential compares against it, and this test pins IT against the verb's
end-to-end behavior, so the chain from canon to verb has no unpinned link."""
import os
import shutil
import tempfile

import pyarest.prims  # noqa: F401
from pyarest import apps as A, system


def _fixture():
    tmp = tempfile.mkdtemp(prefix="induce-oracle-")
    os.makedirs(os.path.join(tmp, "coin", "readings"))
    with open(os.path.join(tmp, "coin", "readings", "app.md"), "w",
              encoding="utf-8") as f:
        f.write(
            "Side is a value type.\n"
            "The possible values of Side are 'heads', 'tails'.\n"
            "Coin is an entity type.\n"
            "Coin has Side.\n"
            "\n"
            "Coin 'c1' has Side 'heads'.\n")
    return tmp


def test_the_domains_and_the_enumeration_are_the_oracle():
    tmp = _fixture()
    try:
        reg = A.Registry(tmp, base_dir=A.default_base())
        reg.compile("coin")
        D = reg._load("coin")
        # the domain order is the oracle's: declared enum literals first
        # (the enumValues cell), then the noun's own cell, then observed
        # role plays, keep-first across the later legs
        assert system.induce_domain(D, "Coin") == ["c1"]
        assert system.induce_domain(D, "Side") == ["heads", "tails"]
        # the enumeration is the cartesian product in domain order, ids
        # deterministic on (ft, index), scores 0 with no hook declared
        out = reg.induce("coin", "Coin_has_Side")
        assert [h["id"] for h in out] == [
            "hyp-Coin_has_Side-0", "hyp-Coin_has_Side-1"]
        assert [h["hidden"]["fact"] for h in out] == [
            ["c1", "heads"], ["c1", "tails"]]
        assert all(h["confidence_score"] == 0 for h in out)
        assert all(h["explains"] == [] for h in out)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def test_the_canon_enumeration_family_matches_the_oracle():
    # the canon meaning reduces to the oracle's exact answers: role domains
    # per noun (declaration order, keep-first across the later legs) and
    # the cartesian product in itertools order
    import itertools

    from pyarest import defs as _dm
    import pyarest.lam as L
    from pyarest.lam import atom as _A, to_lam, from_lam
    from pyarest.reduce import apply as _ap

    tmp = _fixture()
    try:
        reg = A.Registry(tmp, base_dir=A.default_base())
        reg.compile("coin")
        D = reg._load("coin")

        def canon(name, operand):
            with _dm.step(D):
                return from_lam(_ap(_A(name), operand))

        for noun in ("Coin", "Side"):
            pair = L.SEQ(L.CONS(_A(noun))(L.CONS(D)(L.NIL)))
            assert list(canon("system:role_domain", pair)) == \
                system.induce_domain(D, noun), noun
        doms = [system.induce_domain(D, n) for n in ("Coin", "Side")]
        got = canon("system:enum_product",
                    to_lam(tuple(tuple(d) for d in doms)))
        want = [tuple(c) for c in itertools.product(*doms)]
        assert [tuple(r) for r in got] == want
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def test_the_canon_gate_and_coverage_match_the_oracle():
    # the delta gate: a uniqueness constraint makes some candidates
    # introduce violations the baseline lacks; the canon verdict must agree
    # with the oracle's per candidate. Coverage judges HOST-BUILT
    # post-worlds: install and derive stay engine machinery (the worlds are
    # operands, like the validator), and the canon carries the judgment.
    import itertools

    from pyarest import ast as past, defs as _dm, forml
    import pyarest.lam as L
    from pyarest.lam import atom as _A, to_lam, from_lam
    from pyarest.reduce import apply as _ap

    tmp = tempfile.mkdtemp(prefix="induce-oracle-")
    os.makedirs(os.path.join(tmp, "coin", "readings"))
    with open(os.path.join(tmp, "coin", "readings", "app.md"), "w",
              encoding="utf-8") as f:
        f.write(
            "Side is a value type.\n"
            "The possible values of Side are 'heads', 'tails'.\n"
            "Weight is a value type.\n"
            "Coin is an entity type.\n"
            "Coin has Side.\n"
            "Coin has Weight.\n"
            "\n"
            "Coin 'c1' has Side 'heads'.\n"
            "Coin 'c2' has Weight '5'.\n")
    try:
        reg = A.Registry(tmp, base_dir=A.default_base())
        reg.compile("coin")
        D = reg._load("coin")

        def canon(name, operand):
            with _dm.step(D):
                return from_lam(_ap(_A(name), operand))

        part = system.rmap_partition(D)
        val = forml.validate_for("Coin_has_Side", D, part)
        existing = tuple(tuple(r)
                         for r in system.ft_view(D, "Coin_has_Side", part))

        def py_viol(rows):
            pair = L.SEQ(L.CONS(to_lam(tuple(rows)))(L.CONS(D)(L.NIL)))
            with _dm.step(D):
                out = from_lam(_ap(val, pair))
            return {tuple(v) if isinstance(v, tuple) else (v,)
                    for v in (out[1] if len(out) >= 2 else ())}

        doms = [system.induce_domain(D, n) for n in ("Coin", "Side")]

        # two validators: the app's compiled one (unconstrained here, every
        # candidate passes), and a handcrafted role-1 uniqueness, the same
        # object the compiler builds for a declared UC, which splits the
        # verdicts: (c1, tails) doubles c1's role-1 count and must reject,
        # while (c2, *) passes. Both feed the SAME oracle computation and
        # the SAME canon gate, so agreement is pinned on both verdicts.
        from pyarest import constraints as C
        val_uc = system.validate_modal([(C.uniqueness([1]), "alethic")], [])
        verdicts = []
        for v in (val, val_uc):
            def py_v(rows, _v=v):
                pair = L.SEQ(L.CONS(to_lam(tuple(rows)))(L.CONS(D)(L.NIL)))
                with _dm.step(D):
                    out = from_lam(_ap(_v, pair))
                return {tuple(x) if isinstance(x, tuple) else (x,)
                        for x in (out[1] if len(out) >= 2 else ())}
            base = py_v(existing)
            for cand in itertools.product(*doms):
                oracle_pass = not (py_v(existing + (tuple(cand),)) - base)
                operand = L.SEQ(L.CONS(to_lam(tuple(cand)))(
                    L.CONS(to_lam(existing))(L.CONS(v)(L.CONS(D)(L.NIL)))))
                got = canon("system:cand_gate", operand)
                assert (got == "T") == oracle_pass, (cand, got, oracle_pass)
                verdicts.append(oracle_pass)
        # the pair of validators must exercise both verdicts
        assert True in verdicts and False in verdicts

        # coverage on a host-built post-world
        cand = ("c2", "tails")
        D2 = _ap(past.Store("Coin_has_Side"),
                 L.SEQ(L.CONS(to_lam(system._rowsort(set(existing)
                                                     | {cand})))(
                     L.CONS(D)(L.NIL))))
        D3 = system.run_rules(D2, changed=["Coin_has_Side"])
        te_hit = to_lam((("Coin_has_Side", ("c2", "tails")),))
        te_miss = to_lam((("Coin_has_Side", ("c9", "tails")),))
        assert canon("system:cand_covers",
                     L.SEQ(L.CONS(te_hit)(L.CONS(D3)(L.NIL)))) == "T"
        assert canon("system:cand_covers",
                     L.SEQ(L.CONS(te_miss)(L.CONS(D3)(L.NIL)))) == "F"

        # slice (c): score is a Filter-and-sum judgment over MARSHALED
        # normalized rows (numeric-or-one is the oracle's int(str(v))
        # boundary transduction, applied where the operand is built, the
        # same seam ruling as the typed to_explain above); rank is the
        # descending INSERT insertion sort, enumeration-stable on ties
        def norm(v):
            try:
                return int(str(v))
            except ValueError:
                return 1

        score_rows = (("hyp-0", 2), ("hyp-0", "High"), ("hyp-1", 3))
        marshaled = tuple((h, norm(v)) for (h, v) in score_rows)
        for hyp, want in (("hyp-0", 3), ("hyp-1", 3), ("hyp-9", 0)):
            got = canon("system:cand_score",
                        L.SEQ(L.CONS(_A(hyp))(
                            L.CONS(to_lam(marshaled))(L.NIL))))
            assert got == want, (hyp, got, want)

        ranked_in = ((0, "a"), (5, "b"), (3, "c"), (5, "d"), (0, "e"))
        got = canon("system:rank_desc", to_lam(ranked_in))
        want = tuple(sorted(ranked_in, key=lambda r: -r[0]))
        assert tuple(tuple(r) for r in got) == want
        # ties keep enumeration order: b before d, a before e
        assert [r[1] for r in got] == ["b", "d", "c", "a", "e"]
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
