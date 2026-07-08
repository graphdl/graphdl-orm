"""The entity_view slice's naming layer as SHARED CANON (Samuel,
2026-07-08: not four implementations — the shared lambda source).
system:sqlname composes the existing base ops — slug, the single-token
lex row's lower field, implode — into protocol._sql_name byte for
byte; system:sqlcol carries every naming CHOICE of Registry.get's
column pass (unary strips the noun, ref joins the reference mode,
value names the played type, the dedup ordinal suffixes from 2) over
the one new generic base op strip_prefix. The host mangler is hereby
the twin, not the definition."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, from_lam
from pyarest.protocol import _sql_name
from pyarest.reduce import apply


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _col(kind, noun, ft, name, mode, n):
    return from_lam(apply(A("system:sqlcol"),
                          _S(A(kind), A(noun), A(ft), A(name), A(mode), A(n))))


def test_sqlname_twins_the_host_mangler():
    for s in ("Task Priority", "Order_was_placed_by_Customer",
              "café au lait", "  --  ", "", "A1_b2", "is_started",
              "φ", "9lives", "Tick"):
        got = from_lam(apply(A("system:sqlname"), A(s)))
        assert got == _sql_name(s), (s, got, _sql_name(s))


def test_the_unary_column_strips_the_noun_prefix():
    assert _col("unary", "Task", "Task_is_started", "#", "#", 1) == \
        "is_started"
    # a fact type that does not start with the noun mangles whole
    assert _col("unary", "Task", "Reopened_flag", "#", "#", 1) == \
        "reopened_flag"


def test_the_ref_column_joins_player_and_mode():
    assert _col("ref", "Order", "Order_was_placed_by_Customer",
                "Customer", "nr", 1) == "customer_nr"
    # a ref column needs both bytes — a missing mode is a canon bug
    assert _col("ref", "Order", "Order_was_placed_by_Customer",
                "Customer", "#", 1) == "⊥"


def test_the_value_column_names_the_played_type():
    assert _col("value", "Task", "Task_has_Task_Priority",
                "Task Priority", "#", 1) == "task_priority"
    assert _col("value", "Task", "Task_has_Task_Priority", "#", "#", 1) == "⊥"


def test_the_dedup_ordinal_suffixes_from_two():
    assert _col("value", "Task", "Task_has_Task_Priority",
                "Task Priority", "#", 2) == "task_priority_2"
    assert _col("unary", "Task", "Task_is_started", "#", "#", 3) == \
        "is_started_3"


def test_an_unknown_kind_bottoms():
    assert _col("mystery", "Task", "Task_is_started", "#", "#", 1) == "⊥"


def test_strip_prefix_is_policy_free():
    assert from_lam(apply(A("strip_prefix"),
                          _S(A("Task"), A("Task_is_started")))) == \
        "_is_started"
    assert from_lam(apply(A("strip_prefix"),
                          _S(A("Zebra"), A("Task_is_started")))) == \
        "Task_is_started"


# ---- the classification family over a miniature RMAP store ----

def _D():
    """A store spine carrying the classification's source cells: Task
    absorbs a unary (col 2), a functional played by Task Priority
    (col 3), and one played by the entity Customer (col 4); Customer's
    ref mode comes from refScheme, Widget's from refMode."""
    return _S(
        _S(A("CELL"), A("rmapColumns"),
           _S(_S(A("Task"), A(2), A("Task_is_started")),
              _S(A("Task"), A(3), A("Task_has_Task_Priority")),
              _S(A("Task"), A(4), A("Task_was_filed_by_Customer")))),
        _S(A("CELL"), A("role"),
           _S(_S(A("Task_is_started.1"), A("Task_is_started"), A(1), A("Task")),
              _S(A("Task_has_Task_Priority.1"), A("Task_has_Task_Priority"),
                 A(1), A("Task")),
              _S(A("Task_has_Task_Priority.2"), A("Task_has_Task_Priority"),
                 A(2), A("Task Priority")),
              _S(A("Task_was_filed_by_Customer.1"),
                 A("Task_was_filed_by_Customer"), A(1), A("Task")),
              _S(A("Task_was_filed_by_Customer.2"),
                 A("Task_was_filed_by_Customer"), A(2), A("Customer")))),
        _S(A("CELL"), A("instanceOf"),
           _S(_S(A("Task"), A("ObjectType")),
              _S(A("Customer"), A("ObjectType")),
              _S(A("Widget"), A("ObjectType")))),
        _S(A("CELL"), A("refScheme"), _S(_S(A("Customer"), A("nr")))),
        _S(A("CELL"), A("refMode"), _S(_S(A("Widget"), A("code"))))
    )


def test_ev_colrows_projects_the_nouns_layout_in_order():
    got = from_lam(apply(A("system:ev_colrows"), _S(A("Task"), _D())))
    assert got == (("Task", 2, "Task_is_started"),
                   ("Task", 3, "Task_has_Task_Priority"),
                   ("Task", 4, "Task_was_filed_by_Customer"))


def test_ev_kind_classifies_unary_value_and_ref():
    def k(ft):
        return from_lam(apply(A("system:ev_kind"),
                              _S(A("Task"), A(ft), _D())))
    assert k("Task_is_started") == ("unary", "#")
    # Task Priority is NOT declared an object type in this fixture, so
    # the played type names a plain value column; Customer IS one, so
    # its column is a reference. The python conjunction (entities AND
    # entity_tables) reduces to the entity test alone — entities is a
    # subset of entity_tables by construction.
    assert k("Task_has_Task_Priority") == ("value", "Task Priority")
    assert k("Task_was_filed_by_Customer") == ("ref", "Customer")


def test_ev_refmode_prefers_refscheme_then_refmode_then_id():
    def m(n):
        return from_lam(apply(A("system:ev_refmode"), _S(A(n), _D())))
    assert m("Customer") == "nr"
    assert m("Widget") == "code"
    assert m("Task Priority") == "id"


def test_ev_cols_classifies_names_and_dedups_in_layout_order():
    got = from_lam(apply(A("system:ev_cols"), _S(A("Task"), _D())))
    assert got == (
        ("Task_is_started", "unary", "#", "is_started"),
        ("Task_has_Task_Priority", "value", "Task Priority", "task_priority"),
        ("Task_was_filed_by_Customer", "ref", "Customer", "customer_nr"),
    )


def test_ev_cols_suffixes_a_colliding_base_from_two():
    # two unaries whose stripped names collide: the second takes _2,
    # protocol._entity_columns' seen-count exactly
    D = _S(
        _S(A("CELL"), A("rmapColumns"),
           _S(_S(A("Job"), A(2), A("Job_is_hot")),
              _S(A("Job"), A(3), A("Job__is_hot")))),
        _S(A("CELL"), A("role"),
           _S(_S(A("Job_is_hot.1"), A("Job_is_hot"), A(1), A("Job")),
              _S(A("Job__is_hot.1"), A("Job__is_hot"), A(1), A("Job")))),
        _S(A("CELL"), A("instanceOf"), _S(_S(A("Job"), A("ObjectType"))))
    )
    got = from_lam(apply(A("system:ev_cols"), _S(A("Job"), D)))
    assert got == (("Job_is_hot", "unary", "#", "is_hot"),
                   ("Job__is_hot", "unary", "#", "is_hot_2"))



# ---- the whole view over a populated store ----

def _DV():
    """_D() plus populations: the ft OWN CELLS (get reads _pop_rows,
    never the wide column), an own-table binary Task_blocks_Task with
    Task at both roles, and the Task spine."""
    return _S(
        _S(A("CELL"), A("rmapColumns"),
           _S(_S(A("Task"), A(2), A("Task_is_started")),
              _S(A("Task"), A(3), A("Task_has_Task_Priority")),
              _S(A("Task"), A(4), A("Task_was_filed_by_Customer")))),
        _S(A("CELL"), A("role"),
           _S(_S(A("Task_is_started.1"), A("Task_is_started"), A(1), A("Task")),
              _S(A("Task_has_Task_Priority.1"), A("Task_has_Task_Priority"),
                 A(1), A("Task")),
              _S(A("Task_has_Task_Priority.2"), A("Task_has_Task_Priority"),
                 A(2), A("Task Priority")),
              _S(A("Task_was_filed_by_Customer.1"),
                 A("Task_was_filed_by_Customer"), A(1), A("Task")),
              _S(A("Task_was_filed_by_Customer.2"),
                 A("Task_was_filed_by_Customer"), A(2), A("Customer")),
              _S(A("Task_blocks_Task.1"), A("Task_blocks_Task"),
                 A(1), A("Task")),
              _S(A("Task_blocks_Task.2"), A("Task_blocks_Task"),
                 A(2), A("Task")))),
        _S(A("CELL"), A("instanceOf"),
           _S(_S(A("Task"), A("ObjectType")),
              _S(A("Customer"), A("ObjectType")))),
        _S(A("CELL"), A("refScheme"), _S(_S(A("Customer"), A("nr")))),
        _S(A("CELL"), A("factType"),
           _S(_S(A("Task_is_started"), A("{0} is started")),
              _S(A("Task_has_Task_Priority"), A("{0} has {1}")),
              _S(A("Task_was_filed_by_Customer"), A("{0} was filed by {1}")),
              _S(A("Task_blocks_Task"), A("{0} blocks {1}")))),
        _S(A("CELL"), A("Task"), _S(_S(A("t1")), _S(A("t2")), _S(A("t3")))),
        _S(A("CELL"), A("Task_is_started"), _S(_S(A("t1")))),
        _S(A("CELL"), A("Task_has_Task_Priority"),
           _S(_S(A("t1"), A("p0")), _S(A("t1"), A("p2")))),
        _S(A("CELL"), A("Task_was_filed_by_Customer"),
           _S(_S(A("t2"), A("c9")))),
        _S(A("CELL"), A("Task_blocks_Task"),
           _S(_S(A("t1"), A("t2")), _S(A("t3"), A("t1"))))
    )


def test_ev_fields_keys_and_values_mirror_get():
    got = from_lam(apply(A("system:ev_fields"), _S(A("Task"), A("t1"), _DV())))
    # unary key = the sql column; binary key = the played type; the
    # last row wins a repeated key (python's dict-build), absent = "#"
    assert got == (("is_started", "T"),
                   ("Task Priority", "p2"),
                   ("Customer", "#"))


def test_ev_facts_scans_own_tables_at_every_noun_position():
    got = from_lam(apply(A("system:ev_facts"), _S(A("Task"), A("t1"), _DV())))
    assert got == (("Task_blocks_Task", ("t1", "t2")),
                   ("Task_blocks_Task", ("t3", "t1")))
    # t2 plays only position 2 of the first row
    got = from_lam(apply(A("system:ev_facts"), _S(A("Task"), A("t2"), _DV())))
    assert got == (("Task_blocks_Task", ("t1", "t2")),)


def test_entity_view_assembles_exists_fields_facts():
    got = from_lam(apply(A("system:entity_view"),
                         _S(A("Task"), A("t1"), _DV())))
    assert got[0] == "T"
    assert got[1] == (("is_started", "T"),
                      ("Task Priority", "p2"),
                      ("Customer", "#"))
    assert got[2] == (("Task_blocks_Task", ("t1", "t2")),
                      ("Task_blocks_Task", ("t3", "t1")))


def test_entity_view_exists_rides_the_spine_alone():
    # t3 has no absorbed fields and no own facts at... t3 DOES block t1;
    # use a spine-only phantom t9? not in spine either. t3: facts exist.
    # A raw spine member with no facts anywhere: add t9 via the spine
    # in a variant store.
    got = from_lam(apply(A("system:entity_view"),
                         _S(A("Task"), A("t9"), _DV())))
    assert got[0] == "F"
    assert got[1] == (("is_started", "F"),
                      ("Task Priority", "#"),
                      ("Customer", "#"))
    assert got[2] == ()


def test_the_host_view_twins_the_canon_definition():
    # the demotion pin: pyarest.protocol.get_view is hereby the
    # certified-equal OVERRIDE of system:entity_view — exists and
    # fields byte-for-byte (T/F/# to True/False/None), facts as sets
    # (the canon iterates the factType cell's order, the host
    # rmap_partition's; the envelope never promised an order)
    from pyarest.protocol import get_view
    m = {"T": True, "F": False, "#": None}
    for tid in ("t1", "t2", "t3", "t9"):
        canon = from_lam(apply(A("system:entity_view"),
                               _S(A("Task"), A(tid), _DV())))
        seen, fields, facts = get_view(_DV(), "Task", tid)
        assert (canon[0] == "T") == bool(seen), tid
        assert {k: m.get(v, v) for (k, v) in canon[1]} == fields, tid
        assert {(ft, tuple(r)) for (ft, r) in canon[2]} == \
            {(f["fact_type"], tuple(f["row"])) for f in facts}, tid
