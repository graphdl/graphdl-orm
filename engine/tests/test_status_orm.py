"""The status ORM-ification (whitepaper Prop. onestep, Prop. derive): a state
machine's status(e) is the "is currently in Status" fact type, its value the
transition-fold over the entity's events, materialized per RMAP as a status
column on the Object Type's table, not the hand-maintained noun_status cell.
status_facts is the compile-side generator: it gives status(e) its fact type
through the ordinary reading path (so the name is compiled, never hand-built)
plus a marker so the guarded step finds it."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system

MACHINE = """Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Customer places Order.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
"""


def test_status_facts_generates_the_status_fact_type_as_a_column():
    D, _ = forml.compile_model(MACHINE)
    fts = {r[0] for r in system._pop_rows(D, "factType")}
    assert "Order_is_currently_in_Status" not in fts       # corpus shape: no status ft
    D2 = system.status_facts(D)
    # the governed Object Type's status fact type now exists (name compiled)
    assert "Order_is_currently_in_Status" in \
        {r[0] for r in system._pop_rows(D2, "factType")}
    # functional, so RMAP absorbs it as a status COLUMN on Order's table
    part = system.rmap_partition(D2)
    assert part["Order_is_currently_in_Status"] == "Order"
    assert "Order_is_currently_in_Status" in system.table_columns(part, "Order")
    # a marker links the governed Object Type to its status fact type, so the
    # guarded step looks it up rather than reconstructs the name
    assert ("Order", "Order_is_currently_in_Status") in \
        {tuple(r) for r in system._pop_rows(D2, "smStatusFt")}


def test_status_facts_is_a_noop_without_a_machine():
    D, _ = forml.compile_model("Ticket is an entity type.\nStatus is a value type.\n"
                               "Ticket has Status.\n")
    D2 = system.status_facts(D)
    assert {tuple(r) for r in system._pop_rows(D2, "smStatusFt")} == set()


def test_machine_advances_status_in_the_rmap_column():
    """The whole unit, end to end: with the status fact type generated and laid
    out, the machine advances status(e) INTO the Object Type's RMAP status
    column (read and written via the row), and noun_status is not used. This is
    the parity target against test_machine_from_M's noun_status behavior."""
    from pyarest import defs
    from pyarest.lam import to_lam, atom as A
    from pyarest.reduce import apply as _ap
    D, _ = forml.compile_model(MACHINE)
    D = system.status_facts(D)
    D = system.layout_cells(D)
    part = system.rmap_partition(D)

    def create(d, ft, fact):
        with defs.step(d):
            return _ap(A(2), system.create(d, ft, to_lam(fact)))  # ⟨o, D'⟩ → D'
    # o1 starts In Cart, as the status fact in its row column
    D = create(D, "Order_is_currently_in_Status", ("o1", "In Cart"))
    assert ("o1", "In Cart") in system.ft_view(D, "Order_is_currently_in_Status", part)
    # firing the trigger advances o1 to Placed, still in the column
    D = create(D, "Customer_places_Order", ("c1", "o1"))
    status = system.ft_view(D, "Order_is_currently_in_Status", part)
    assert ("o1", "Placed") in status
    assert ("o1", "In Cart") not in status
    # the noun_status wart does not exist: the column IS the store of record
    assert {tuple(r) for r in system._pop_rows(D, "Order_status")} == set()


def test_machine_app_advances_status_in_the_column_through_the_registry(tmp_path):
    """The production path: a machine app compiled through the Registry pipeline
    (status_facts wired before replay) advances status(e) into the RMAP column,
    queryable as the "is currently in Status" fact type -- no noun_status cell."""
    import os
    from pyarest import apps
    root = str(tmp_path)
    d = os.path.join(root, "orders", "readings")
    os.makedirs(d)
    with open(os.path.join(d, "m.md"), "w", encoding="utf-8") as f:
        f.write(MACHINE)
    reg = apps.Registry(root)
    reg.compile("orders")
    # status_facts ran in the pipeline, so the status fact type exists to apply into
    reg.apply("orders", "Order_is_currently_in_Status", ("o1", "In Cart"))
    reg.apply("orders", "Customer_places_Order", ("c1", "o1"))
    assert set(reg.query("orders", "Order_is_currently_in_Status")) == {("o1", "Placed")}
