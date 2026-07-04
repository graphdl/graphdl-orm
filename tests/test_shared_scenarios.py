"""The cross-host case table (shared/scenarios.py, intersection source): every
host reduces each case's expr applied to its operand. Here the PYTHON half:
the Scott evaluator (the ground truth) and the delta fast path must answer
every case identically, which makes the table the intra-Python differential
too. The C# and Java hosts consume the same bytes through their wraps and
print name=result for the cross-host comparison."""
import pyarest.prims  # noqa: F401
from pyarest import canon, reduce as R
from pyarest.lam import from_lam


def _cases():
    return canon.read("scenarios.py")


def test_the_case_table_reads_and_covers_the_semantics():
    cases = _cases()
    assert len(cases) >= 38
    names = [n for (n, _) in cases]
    assert "case:lt-mixed" in names and "case:while-count" in names


def test_scott_and_delta_agree_on_every_case():
    import os
    results = {}
    for name, pair in _cases():
        expr_operand = from_lam(pair)
        assert isinstance(expr_operand, tuple) and len(expr_operand) == 2
    # evaluate through BOTH evaluators via the seam
    from pyarest import reduce as _r
    for evaluator in ("lambda", "delta"):
        _r.use_evaluator(evaluator)
        try:
            for name, pair in _cases():
                # pair is ⟨expr, operand⟩ as one Scott value: select and apply
                from pyarest.lam import atom as A
                expr = _r.apply(A(1), pair)
                operand = _r.apply(A(2), pair)
                got = from_lam(_r.apply(expr, operand))
                results.setdefault(name, {})[evaluator] = got
        finally:
            _r.use_evaluator("delta")
    diverged = {n: r for n, r in results.items()
                if r["lambda"] != r["delta"]}
    assert diverged == {}, f"evaluator divergence: {sorted(diverged)[:5]}"
