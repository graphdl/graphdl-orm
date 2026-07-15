"""G3, the case-table leg (SPEC H6, §13 G3): one canon, two evaluators,
byte-identical answers.

The Rust host include!s /arest.canon and /scenarios.canon and, under
--cases, reduces every scenario pair through ITS μ and prints name=result.
The Python host computes the same cases through ITS reducer over the same
bytes. The lines must agree exactly — the show convention is shared by
every host (transcribed from the quarry's cross-host differential,
test_csharp_kernel.py).
"""
import os
import subprocess
import sys

import pytest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_BIN = os.path.join(_ROOT, "host_rs", "target", "release",
                    "arest.exe" if os.name == "nt" else "arest")


def _show(o):
    if isinstance(o, tuple):
        return "(" + ", ".join(_show(x) for x in o) + ")"
    if isinstance(o, str):
        return "'" + o + "'"
    return str(o)


def _python_cases():
    from host_py import canon
    from host_py import reduce as _r
    from host_py.lam import atom as A, from_lam
    out = {}
    for name, pair in canon.read("scenarios.canon"):
        expr = _r.apply(A(1), pair)
        operand = _r.apply(A(2), pair)
        got = from_lam(_r.apply(expr, operand))
        # from_lam renders bottom as the bare character; hosts print it bare
        out[name] = "⊥" if got == "⊥" else _show(got)
    return out


@pytest.mark.skipif(not os.path.exists(_BIN),
                    reason="host_rs not built (cargo build --release)")
def test_the_hosts_agree_on_the_case_table():
    out = subprocess.run([_BIN, "--cases"], capture_output=True, text=True,
                         timeout=600, encoding="utf-8")
    assert out.returncode == 0, out.stderr[-800:]
    lines = {l.split("=", 1)[0]: l.split("=", 1)[1]
             for l in out.stdout.splitlines() if "=" in l}
    want = _python_cases()
    assert want, "the Python side read no scenarios"
    missing = [n for n in want if n not in lines]
    assert missing == [], f"host missing cases: {missing[:5]}"
    diverged = {n: (lines[n], want[n]) for n in want if lines[n] != want[n]}
    assert diverged == {}, f"cross-host divergence: {dict(list(diverged.items())[:3])}"
