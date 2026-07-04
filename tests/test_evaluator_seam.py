"""Stratum 1 of the polyglot debug: the kernel's evaluator choice is an
EXPLICIT, switchable binding — the one seam that cannot ride DEFS (rho is
implemented by apply), so it gets the same shape one level down: canonical
lambda kernel as ground truth, delta as the registered fast path, selection by
name. The oracle suites stop importing around the seam and switch it."""
import pyarest.prims  # noqa: F401
from pyarest import reduce as R
from pyarest.lam import to_lam, from_lam, atom as A


def test_the_evaluator_switches_by_name_and_agrees():
    probe = to_lam((1, 2, 3))
    tl = R.apply(A(2), probe)                                 # delta (the default)
    try:
        R.use_evaluator("lambda")
        lam = R.apply(A(2), probe)                            # the pure kernel
    finally:
        R.use_evaluator("delta")
    assert from_lam(tl) == from_lam(lam) == 2
    assert R.active_evaluator() == "delta"


def test_unknown_evaluator_names_are_refused():
    try:
        R.use_evaluator("cuda")
        raised = False
    except (KeyError, ValueError):
        raised = True
    assert raised
