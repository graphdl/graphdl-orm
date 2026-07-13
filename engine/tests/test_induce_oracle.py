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
