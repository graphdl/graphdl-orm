"""Stratum 2 of the polyglot debug: the NATIVE vocabulary is the third consumer
of the intersection files (CPython exec over Scott builders, rustc include!,
and now exec over delta-carrier builders). Faithfulness is proven the
differential way before registration is even wired: for every definition in
every shared file, the native-built object must equal what the Scott boundary
would convert the canonical object to — byte-identical trees, no new
representation, just the bridge pre-empted at load time."""
import pyarest.prims  # noqa: F401
from pyarest import canon, delta


def test_the_native_vocabulary_matches_the_boundary_conversion():
    for fname in ("arest.canon",):
        canonical = dict(canon.read(fname))
        native = dict(canon.read_native(fname))
        assert set(native) == set(canonical), fname
        for name, obj in canonical.items():
            assert native[name] == delta.scott_to_native(obj), (fname, name)


def test_load_registers_native_twins_and_delta_uses_them():
    # the registration wiring: canon.load registers BOTH representations, and
    # delta's store rebuild takes the native twin instead of converting the
    # Scott object — the conversion of ~120 canonical defs otherwise recurs on
    # EVERY defs.version bump (each DefineIn during an ingest)
    from pyarest import defs, reduce as R
    from pyarest.lam import to_lam, from_lam, atom as A
    canon.load_all()
    assert "theta:proj1" in defs.native
    assert defs.native["theta:proj1"] == delta.scott_to_native(
        defs.latest["theta:proj1"][1])
    rows = to_lam((("a", 1), ("b", 2), ("a", 1)))
    got = from_lam(R.apply(A("theta:proj1"), rows))
    assert sorted(got) == [("a",), ("b",)]


def test_native_s_constructors_keep_exact_arity():
    v = canon.vocabulary_native([])
    try:
        v["S3"]("a", "b")
        raised = False
    except TypeError:
        raised = True
    assert raised
    assert v["S2"](v["A"]("COMP"), v["N"](1)) == ("COMP", 1)
    assert v["K"](v["A"]("x")) == ("CONST", "x")
    assert v["PHI"]() == ()
