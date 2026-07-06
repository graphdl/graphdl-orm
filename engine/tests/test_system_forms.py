"""§14.5 system forms and §14.4.4 supervision, across §14.7 tenant cells. PIPE builds a
system from component systems: the composite transition matches A's output to B's input,
each component stepping under ITS OWN store (a tenant cell of the composite), and a
component ERROR aborts the composite step with the composite store unchanged (§14.3.1
lifted). SUPERVISE is delegation with reclaim: the child steps under fuel, a runaway
answers ⟨ERROR, unchanged⟩ with the child store intact, and the parent installs or
retires the child's SYSTEM by ordinary stores — supervision is nesting plus fuel."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, defs
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _D(*cells):
    l = L.NIL
    for c in reversed(cells):
        l = L.CONS(c)(l)
    return L.SEQ(l)


def _sys(f):
    return S(A("CONS"), f, A("DEFS"))                         # SYSTEM:x = ⟨f(x), D⟩


SUCC = S(A("COMP"), A("+"), S(A("CONS"), A("id"), S(A("CONST"), A(1))))
DBL = S(A("COMP"), A("*"), S(A("CONS"), A("id"), S(A("CONST"), A(2))))
SPIN = S(A("WHILE"), S(A("CONST"), A("T")), A("id"))


def test_pipe_matches_a_output_to_b_input():
    D = _D(ast.cell("A", _D(ast.cell("SYSTEM", _sys(SUCC)))),
           ast.cell("B", _D(ast.cell("SYSTEM", _sys(DBL)))))
    (o, _Dp) = from_lam(ast.pipe(A(1), D))
    assert o == 4                                             # (1+1)*2


def test_a_component_error_aborts_the_composite():
    D = _D(ast.cell("A", _D(ast.cell("SYSTEM", _sys(SUCC)))),
           ast.cell("B", _D()))                               # B has no SYSTEM
    (o, Dp) = from_lam(ast.pipe(A(1), D))
    assert o == "ERROR" and Dp == from_lam(D)                 # composite unchanged


def test_supervised_runaway_child_reclaims_control_with_child_intact():
    child = _D(ast.cell("SYSTEM", SPIN))
    D = _D(ast.cell("CHILD", child))
    (o, Dp) = from_lam(ast.supervise(A(7), D, fuel=10000))
    assert o == "ERROR" and Dp == from_lam(D)                 # reclaim; child untouched
    # RESET cannot repair a divergent child (§14.3.2 runs the child's OWN transition);
    # the parent installs the new SYSTEM directly — the child is a cell of ITS store
    D2 = ast.child_install(D, "CHILD", "SYSTEM", _sys(SUCC))
    (o2, _D3) = from_lam(ast.supervise(A(7), D2, fuel=200000))
    assert o2 == 8                                            # repaired child answers


def test_retiring_a_child_empties_its_cell():
    D = _D(ast.cell("CHILD", _D(ast.cell("SYSTEM", _sys(SUCC)))))
    D = ast.child_retire(D, "CHILD")
    (o, _Dp) = from_lam(ast.supervise(A(7), D, fuel=100000))
    assert o == "ERROR"                                       # nothing there to run
