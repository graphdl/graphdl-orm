"""The enumerable boundary is live on BOTH paths (audit A2): a definition registered at
runtime — the paper's platform binding: render, httpFetch, upsert — must be reachable by
the δ evaluator exactly as by the λ kernel, and registering must invalidate the δ cache."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import reduce as R
from pyarest import defs


def test_runtime_registered_function_reaches_both_paths():
    defs.register("render#t", lambda mu: lambda o: L.atom("WIDGET"))
    y = to_lam(("fact",))
    assert from_lam(R.apply_lambda(A("render#t"), y)) == "WIDGET"
    assert from_lam(R.apply(A("render#t"), y)) == "WIDGET"      # δ path sees the boundary


def test_registered_function_receives_a_working_mu():
    # the impl(mu)(operand) contract: the host lambda may reduce sub-expressions via mu
    from pyarest.reduce import mkapp
    defs.register("twice#t", lambda mu: lambda o: mu(mkapp(A("tl"))(mu(mkapp(A("tl"))(o)))))
    y = to_lam(("a", "b", "c"))
    assert from_lam(R.apply_lambda(A("twice#t"), y)) == ("c",)
    assert from_lam(R.apply(A("twice#t"), y)) == ("c",)


def test_register_invalidates_the_delta_cache():
    # apply once (warms the δ store), register a NEW def, apply again — it must be visible
    y = to_lam(("x",))
    assert from_lam(R.apply(A(1), y)) == "x"                     # warm the cache
    defs.register("late#t", lambda mu: lambda o: L.atom("LATE"))
    assert from_lam(R.apply(A("late#t"), y)) == "LATE"           # not stale ⊥


def test_boundary_query_lists_registered_only():
    defs.register("effect#t", lambda mu: lambda o: o)
    b = defs.boundary()
    assert "effect#t" in b and "tl" in b                         # registered = the boundary
    assert "run" not in b                                        # compiled defs are above it
