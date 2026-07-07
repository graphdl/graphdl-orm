"""The Java host as the fourth fleet member: gen_canon.py byte-wraps the
shared intersection files into a varargs call (the include! equivalent),
javac compiles the same bytes, and the differential holds the Java 8 reducer
to the Python evaluator's answers on the twin-test cases. Skips cleanly
where the JDK is absent."""
import os
import subprocess

import pytest

import pyarest.prims  # noqa: F401
from pyarest.lam import atom as A, from_lam, to_lam
from pyarest.reduce import apply as _ap

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_JAVA = os.path.join(_ROOT, "java")
_JDK = r"C:\Program Files\Java\jdk1.8.0_211\bin"


def _show(o):
    if isinstance(o, tuple):
        return "(" + ", ".join(_show(x) for x in o) + ")"
    if isinstance(o, str):
        return "'" + o + "'"
    return str(o)


@pytest.mark.skipif(not os.path.exists(_JDK), reason="no JDK")
def test_the_java_kernel_agrees_with_the_python_evaluator():
    subprocess.run(["python", os.path.join(_JAVA, "gen_canon.py")],
                   check=True, capture_output=True, cwd=_JAVA)
    os.makedirs(os.path.join(_JAVA, "out"), exist_ok=True)
    subprocess.run([os.path.join(_JDK, "javac.exe"), "-encoding", "UTF-8",
                    "-d", "out", "Vocab.java", "Canon.java", "Reducer.java",
                    "Program.java"],
                   check=True, capture_output=True, cwd=_JAVA, timeout=300)
    out = subprocess.run([os.path.join(_JDK, "java.exe"),
                          "-Dfile.encoding=UTF-8", "-cp", "out",
                          "arestlam.Program"],
                         capture_output=True, text=True, timeout=300,
                         encoding="utf-8", cwd=_JAVA)
    assert out.returncode == 0, out.stderr[-800:]
    lines = {l.split("=", 1)[0]: l.split("=", 1)[1]
             for l in out.stdout.splitlines() if "=" in l}
    assert int(lines["defs"]) >= 106

    max2 = from_lam(_ap(A("system:max2"), to_lam(("305", "1190"))))
    assert lines["max2('305','1190')"] == _show(max2) == "'1190'"

    pops = to_lam(((("t1", "draft"), ("t2", "review")),
                   (("t1", "submit"), ("t2", "approve")),
                   (("t1", "review"), ("t2", "done"))))
    sm = from_lam(_ap(A("system:sm_join"), pops))
    assert lines["sm_join"] == _show(sm)

    mint = from_lam(_ap(_ap(A("system:mint_next"), A(1)),
                        to_lam((("2", "x"), ("7", "y"), ("4", "z")))))
    assert lines["mint_next"].split(") ", 1)[1] == _show(mint) == "8"

    from pyarest.lam import SEQ, NIL, CONS
    rule = _ap(A("system:join_rule"), to_lam((2, (1, 3))))
    closure = from_lam(_ap(_ap(A("system:derive_of"), SEQ(CONS(rule)(NIL))),
                           to_lam((("a", "b"), ("b", "c"), ("c", "d")))))
    assert lines["closure"] == _show(closure)
    _assert_cases_match(lines)


def _python_cases():
    from pyarest import canon
    from pyarest import reduce as _r
    from pyarest.lam import atom as A
    out = {}
    for name, pair in canon.read("scenarios.canon"):
        expr = _r.apply(A(1), pair)
        operand = _r.apply(A(2), pair)
        got = from_lam(_r.apply(expr, operand))
        # from_lam renders bottom as the bare character; hosts print it bare
        out[name] = "⊥" if got == "⊥" else _show(got)
    return out


def _assert_cases_match(lines):
    want = _python_cases()
    missing = [n for n in want if n not in lines]
    assert missing == [], f"host missing cases: {missing[:5]}"
    diverged = {n: (lines[n], want[n]) for n in want if lines[n] != want[n]}
    assert diverged == {}, f"cross-host divergence: {dict(list(diverged.items())[:3])}"
