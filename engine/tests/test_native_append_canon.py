"""#20 certified-equal twin: engine.run_append (the self-host compile g-loop's plain assert
as a native store-append) reproduces the lambda oracle α₂(run(to_lam(fact), D, cell_name=cell))
exactly. The g-loop consumes only D′ per assert; run_append computes it directly instead of
reducing the whole build_system pipeline over the base-sized D each time (the #20 compile hot
path). The lambda `run` stays canonical — run_append defers to it for any non-plain shape.

Doctrine (Samuel, #17/#18): the host keeps native twins for performance; the lambda is the
meaning. This test IS the acceptance differential — decoded populations compared with ==."""
import pyarest.prims  # noqa: F401
from pyarest import compiler, engine as eng
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest.reduce import apply as R


def _oracle(D, cell, fact):
    return from_lam(R(A(2), eng.run(to_lam(fact), D, cell_name=cell)))


def _twin(D, cell, fact):
    return from_lam(eng.run_append(fact, D, cell))


def _small_D():
    model = "\n".join([
        "Person is an entity type.", "Company is an entity type.",
        "Person works for Company.", "Person has Name.",
        "Person 1 works for Company 2.", "Person 3 works for Company 2.",
    ])
    D, _ = compiler.compile_model(model, D=None)
    return D


CASES = [
    ("factType", ("Widget", "has", "Color")),               # append to a populated meta cell
    ("factType", ("Person", "works for", "Company")),        # DUPLICATE -> dedup keeps one
    ("brandNewCell", ("x", "y")),                            # absent cell -> fresh
    ("Person works for Company", ("Person 9", "Company 9")),  # instance population
    ("Person works for Company", ("Person 1", "Company 2")),  # duplicate instance
    ("subtypeOf", ("A",)),                                   # unary row
    ("factType", ("N", 42)),                                 # numeric atom (NATEQ vs ==)
]


def test_run_append_twins_oracle():
    D = _small_D()
    for cell, fact in CASES:
        assert _twin(D, cell, fact) == _oracle(D, cell, fact), (cell, fact)


def test_run_append_threads():
    # chaining appends (as the g-loop does): the twin's D′ must feed the next append
    # identically to the lambda's D′
    D = _small_D()
    seq = [("factType", ("A", "r", "B")), ("factType", ("A", "r", "B")),  # dup second
           ("factType", ("C", "s", "D")), ("subtypeOf", ("A", "B"))]
    Dt = Do = D
    for cell, fact in seq:
        Dt = eng.run_append(fact, Dt, cell)
        Do = R(A(2), eng.run(to_lam(fact), Do, cell_name=cell))
    for probe in ("factType", "subtypeOf"):
        assert from_lam(eng.run_append(("z", "z"), Dt, probe)) == \
               from_lam(R(A(2), eng.run(to_lam(("z", "z")), Do, cell_name=probe)))
