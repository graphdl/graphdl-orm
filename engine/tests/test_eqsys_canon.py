"""eq. sys from the shared source: SYSTEM : ⟨⟨entity, op⟩, D⟩ applies the handler
that D itself holds for the entity (fetched by RUNTIME name, reflected by rho) to
⟨op, D⟩ — the whole engine as one lambda over values. DynFetch answers # for an
address naming no cell, and #:x reduces to bottom, so wrong-tenant access is not
forbidden but impossible (Prop. tenant). The strict gates hand-build stores holding
handler cells and dispatch through the canonical names; the host module's SYSTEM
must be the canon value, not a fork."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import ast, defs
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


ECHO_OP = S(A("CONS"), S(A("CONST"), A("did")), A(1))         # handler: op ↦ ⟨did, op⟩


def _D(*cells):
    return to_lam(tuple(("CELL", n, v) for (n, v) in cells))


def test_dynfetch_resolves_runtime_names_or_answers_the_mark():
    D = ast.cell("box", to_lam(("payload",)))
    store = S(D)
    with defs.step(L.SEQ(L.NIL)):
        got = from_lam(apply(A("ast:DynFetch"), S(A("box"), store)))
        missing = from_lam(apply(A("ast:DynFetch"), S(A("zz"), store)))
    assert got == ("payload",)
    assert missing == "#"


def test_system_dispatches_through_the_store_held_handler():
    store = S(ast.cell("orders", ECHO_OP))
    with defs.step(L.SEQ(L.NIL)):
        out = from_lam(apply(A("ast:SYSTEM"),
                             S(S(A("orders"), A("op1")), store)))
        host = from_lam(apply(ast.SYSTEM,
                              S(S(A("orders"), A("op1")), store)))
    assert out == ("did", "op1") == host


def test_an_unknown_entity_is_bottom_not_an_error_value():
    store = S(ast.cell("orders", ECHO_OP))
    with defs.step(L.SEQ(L.NIL)):
        out = from_lam(apply(A("ast:SYSTEM"),
                             S(S(A("ghost"), A("op1")), store)))
    assert out == "⊥"                                          # #:x reduces to bottom


def test_the_host_module_binds_the_canon():
    # the host SYSTEM carries canonical references: loaded, not rebuilt
    tree = from_lam(ast.SYSTEM)
    def refs(t):
        if isinstance(t, tuple):
            return any(refs(x) for x in t)
        return isinstance(t, str) and t.startswith("ast:")
    assert refs(tree)
