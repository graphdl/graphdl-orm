"""build_system from the shared source: the AST transition (paper eq. create,
emit ∘ validate ∘ derive ∘ resolve, committing iff no alethic violation; Backus 14.3's
⟨o, d⟩ pair) as canonical stage definitions assembled by ast:build_system over a
nine-slot options record ⟨cell, validate?, resolve?, derive?, links?, machine?,
mealy?, index?, append?⟩, optional slots the empty sequence or a wrapped value, the
machine slot carrying ⟨status, sm⟩ or ⟨status, sm, entity_role⟩. The gates run the
canonical name against the host composite on identical stores across every leg
combination, plus absolute checks: the default resolve is append-if-absent with
re-assertion the identity, and an alethic flag leaves D untouched while o still
reports the violations."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A, PHI
from pyarest import ast, defs
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _D(*cells):
    return to_lam(tuple(("CELL", n, v) for (n, v) in cells))


_REJECT_ALL = S(A("CONS"), A(1), A(1), S(A("CONST"), A("T")))      # V=P'', flag=T
_SM_P2 = A(2)                                                      # status' = P'' (a pop)


def _slot(v=None):
    return to_lam(()) if v is None else S(v)


def _record(cell, validate=None, resolve=None, derive=None, links=None,
            machine=None, mealy=None, index=None, append=None):
    return S(A(cell), _slot(validate), _slot(resolve), _slot(derive), _slot(links),
             to_lam(()) if machine is None else S(*machine),
             _slot(mealy), _slot(A(index) if index else None),
             _slot(A(append) if append else None))


def _run_both(record, kwargs, I, D):
    with defs.step(L.SEQ(L.NIL)):
        canon_obj = apply(A("ast:build_system"), record)
        got = from_lam(apply(canon_obj, S(I, D)))
        want = from_lam(apply(ast.build_system(**kwargs), S(I, D)))
    assert got == want, f"canonical build_system diverges: {got!r} != {want!r}"
    return got


def test_the_bare_transition_appends_and_commits():
    D = _D(("F", (("a",),)))
    o, d = _run_both(_record("F"), {"cell_name": "F"}, to_lam(("b",)), D)
    assert set(o[0]) == {("a",), ("b",)} and o[1] == ()
    assert ("CELL", "F", (("b",), ("a",))) in d or ("CELL", "F", (("a",), ("b",))) in d
    # re-assertion is the identity (fact-as-function)
    o2, d2 = _run_both(_record("F"), {"cell_name": "F"}, to_lam(("a",)), D)
    assert set(o2[0]) == {("a",)}


def test_an_alethic_flag_refuses_and_reports():
    D = _D(("F", ()))
    o, d = _run_both(_record("F", validate=_REJECT_ALL),
                     {"cell_name": "F", "validate_obj": _REJECT_ALL},
                     to_lam(("x",)), D)
    assert o[1] == (("x",),)                                  # V reported
    assert d == (("CELL", "F", ()),)                          # D unchanged


def test_the_machine_leg_advances_in_the_same_step():
    # the machine slot is the status COLUMN ⟨table, col, width⟩: status' lands
    # as a routed row_overwrite on each governed entity's 3NF row
    D = _D(("F", ()), ("T", (("e1",),)), ("T:e1", ("e1", "s0")))
    slot = ("T", 2, 2)
    o, d = _run_both(
        _record("F", machine=(to_lam(slot), _SM_P2)),
        {"cell_name": "F", "machine": (slot, _SM_P2)},
        to_lam(("e1", "go")), D)
    cells = dict((c[1], c[2]) for c in d)
    assert cells["F"] == (("e1", "go"),)
    assert cells["T:e1"] == ("e1", "go")                      # status' committed with the fact


def test_index_and_append_legs_ride_the_same_commit_chain():
    D = _D(("T", ()), ("T_idx", ()), ("FT", ()))
    o, d = _run_both(
        _record("T", index="T_idx", append="FT"),
        {"cell_name": "T", "index_cell": "T_idx", "append_cell": "FT"},
        to_lam(("k1", "v")), D)
    cells = dict((c[1], c[2]) for c in d)
    assert cells["T"] == (("k1", "v"),)
    assert cells["T_idx"] == (("k1",),)
    assert cells["FT"] == (("k1", "v"),)
    # idempotent on re-write: index and append unchanged
    o2, d2 = _run_both(
        _record("T", index="T_idx", append="FT"),
        {"cell_name": "T", "index_cell": "T_idx", "append_cell": "FT"},
        to_lam(("k1", "v")), to_lam(d))
    cells2 = dict((c[1], c[2]) for c in d2)
    assert cells2["T_idx"] == (("k1",),) and cells2["FT"] == (("k1", "v"),)


def test_the_links_leg_with_the_entity_role():
    # links from the post-step status of the addressed entity; an entity with no
    # status row gets φ, never bottom
    links_obj = S(A("ALPHA"), A(2))                           # each status row -> its status
    sm = S(A("CONST"), to_lam((("e1", "Placed"),)))           # status' population
    slot = ("St", 2, 2)
    D = _D(("F", ()), ("St", ()))
    o, _d = _run_both(
        _record("F", links=links_obj, machine=(to_lam(slot), sm, A(1))),
        {"cell_name": "F", "links_obj": links_obj, "machine": (slot, sm, 1)},
        to_lam(("e1", "place")), D)
    assert o[2] == ("Placed",)
    o2, _d2 = _run_both(
        _record("F", links=links_obj, machine=(to_lam(slot), sm, A(1))),
        {"cell_name": "F", "links_obj": links_obj, "machine": (slot, sm, 1)},
        to_lam(("e9", "place")), D)
    assert o2[2] == ()                                        # no status row: φ


def test_the_mealy_leg_appends_emissions_as_the_last_part():
    mealy = S(A("CONST"), A("receipt"))
    sm = S(A("CONST"), to_lam(()))
    slot = ("St", 2, 2)
    D = _D(("F", ()), ("St", ()))
    o, _d = _run_both(
        _record("F", machine=(to_lam(slot), sm), mealy=mealy),
        {"cell_name": "F", "machine": (slot, sm), "mealy_obj": mealy},
        to_lam(("e1",)), D)
    assert o[-1] == "receipt"


def test_the_stored_absorbed_handler_equals_the_python_create():
    """The write-path flip's first leaf: an ABSORBED fact type's create:<ft>
    handler is stored whole — its nine-slot record computes the fact-dependent
    cell name (cellkey(table, key)) at reduce time; every other slot is the
    spec's constant. Reducing the stored handler over ⟨fact, D⟩ answers the
    same ⟨o, D'⟩ as the python create — including the machine leg advancing
    the status column — so the resident serves absorbed writes natively off
    the cell's presence alone."""
    from pyarest import forml, system
    from pyarest.reduce import apply as _ap
    MODEL = """Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Order is paid by Customer.
Each Order is paid by exactly one Customer.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'pay' is from Status 'In Cart'.
Transition 'pay' is to Status 'Paid'.
Transition 'pay' is triggered by Fact Type 'Order is paid by Customer'.
"""
    D, _ = forml.compile_model(MODEL)
    D = system.layout_cells(system.status_facts(D))
    D = system.create_handlers(D)
    D = apply(A(2), system.create(D, "Order_is_currently_in_Status",
                                  to_lam(("o1", "In Cart"))))
    # the handler cell exists for the ABSORBED trigger now (phase two done)
    handler_rows = [c for c in from_lam(D)
                    if isinstance(c, tuple) and len(c) >= 3
                    and c[1] == "create:Order_is_paid_by_Customer"]
    assert handler_rows, "absorbed create handler not stored"
    handler = to_lam(handler_rows[0][2])
    fact = to_lam(("o1", "c1"))
    with defs.step(D):
        got = from_lam(_ap(handler, S(fact, D)))
    want = from_lam(system.create(D, "Order_is_paid_by_Customer", fact))
    assert got == want                                        # ⟨o, D'⟩ byte-equal
    part = system.rmap_partition(to_lam(got[1]))
    assert ("o1", "Paid") in system.ft_view(
        to_lam(got[1]), "Order_is_currently_in_Status", part)  # machine advanced
