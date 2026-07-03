"""Partitioning across parallel instances per the platform arc: the RMAP table is the
partition unit (the writer model's stream scope), creates route to the owning instance,
and the L0/CALM property is the test — a monotone derivation converges to the same lfp
regardless of which instance owned which facts and in which order the partitions merge
(union of closures = closure of union, now across REAL separate engine processes, the
resident Rust kernels). Skipped cleanly without the built kernel."""
import pytest
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import from_lam, to_lam
from pyarest import cluster, forml, polyglot, system

pytestmark = pytest.mark.skipif(not polyglot.rust_available(),
                                reason="rust kernel not built (cd rust; cargo build --release)")


MODEL = """Person is an entity type.
Glyph is an entity type.
Person mentors Person.
Glyph links Glyph.
Person1 reaches Person2 if Person1 mentors Person2.
Person1 reaches Person3 if Person1 mentors Person2 and Person2 reaches Person3.
"""


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_creates_route_by_partition_and_reads_merge():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    cl = cluster.Partitioned(D, instances=2)
    try:
        cl.create("Person_mentors_Person", ("a", "b"))
        cl.create("Glyph_links_Glyph", ("g1", "g2"))
        assert cl.owner("Person_mentors_Person") != cl.owner("Glyph_links_Glyph") \
            or cl.instances == 1
        merged = cl.merged_cells()
        assert ("a", "b") in merged["Person_mentors_Person"]
        assert ("g1", "g2") in merged["Glyph_links_Glyph"]
    finally:
        cl.close()


def test_calm_convergence_across_real_instances():
    # union of closures = closure of union: derive on each instance over ITS partition,
    # merge, close once more — the lfp is the same as single-instance ground truth
    D, _ = forml.compile_model(MODEL)
    chain = [("a", "b"), ("b", "c"), ("c", "d")]
    truth, _ = forml.compile_model(MODEL)
    from pyarest.reduce import apply
    from pyarest.lam import atom as A
    from pyarest import ast
    truth = apply(ast.Store("Person_mentors_Person"),
                  L.SEQ(L.CONS(to_lam(tuple(chain)))(L.CONS(truth)(L.NIL))))
    truth = system.run_rules(truth)
    want = _cell(from_lam(truth), "Person_reaches_Person")

    cl = cluster.Partitioned(D, instances=2)
    try:
        for i, fact in enumerate(chain):
            cl.create("Person_mentors_Person", fact, spread=i)  # deliberately scatter
        got = cl.derive_merged("Person_reaches_Person")
    finally:
        cl.close()
    assert got == want == {("a", "b"), ("b", "c"), ("c", "d"),
                           ("a", "c"), ("b", "d"), ("a", "d")}
