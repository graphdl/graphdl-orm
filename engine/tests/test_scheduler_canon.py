"""The scheduler's first data-fication (the last L1 chip, first slice):
WHICH pass each derived head belongs to becomes a compile-time layout cell
— passHeads rows ⟨pass, head⟩, computed once beside rmapColumns — instead
of the kindmap/keyspans/self_supporting classification both hosts
recompute inside every run_rules call. The pass kinds mirror the joint
fixpoint's strata (engine.py run_rules; rust op_run_rules): 'agg' for
aggregate-rule heads, 'keyed' for key-spanned plain-ruled heads (the
task-955 upsert; kind-blind, like the pass), 'sweep' for derivation-OWNED
acyclic plain heads (delete-and-rederive, GMS93), 'dred' for the
self-supporting ones (empty-first refill). Derivation-OWNED means the
NORMA storage kinds whose population no user asserts — fully-derived (*)
and derived-and-stored (**); semi-derived (+/++) and unmarked ruled heads
keep asserted rows and stay out of the destructive passes. The ** half of
that set is the 2026-07-08 resolution of the staged open question: NORMA
says ** is "derive materializes into the cell, kept in sync", and gating
sweeps on * only had left every non-keyed ** head (the tasks board's
recommendation columns, the claude app's deontic trigger) silently dead
since the 0.9.0 swap. The schedule becomes visible data; the pass BODIES
stay native (the certified fast lane); the ORDER and MEMBERSHIP move to
the store, the pipeline-as-data endgame's first face."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system
from pyarest.engine import _pop_rows

MODEL = """Task(.id) is an entity type.
Peer(.id) is an entity type.
Cost is a value type.
Rank is a value type.
Peer serves Task.
Peer has Cost.
Peer has Rank.
Task blocks Task.
Task has Cost. **
Task has Rank. **
Each Task has at most one Rank.
Task is reachable. **
Task is urgent.

* Task has Cost iff some Peer serves that Task and that Peer has Cost.
* Task has Rank iff some Peer serves that Task and that Peer has Rank.
* Task is reachable iff the Task blocks some Task1 and Task1 has Cost.
* Task is reachable iff the Task blocks some Task1 and Task1 is reachable.
* Task is urgent iff the Task blocks some Task1 and Task1 has Cost.
"""

INSTANCES = """
Task 't1' blocks Task 't2'.
Peer 'p1' serves Task 't1'.
Peer 'p2' serves Task 't2'.
Peer 'p1' has Cost '5'.
Peer 'p2' has Cost '7'.
Peer 'p1' has Rank '1'.
"""


def _passheads(model=MODEL):
    D, _rep = forml.compile_model(model)
    D = system.scheduler_cells(D)
    return {(r[0], r[1]) for r in _pop_rows(D, "passHeads") if len(r) >= 2}


def test_the_pass_table_classifies_every_derived_head():
    got = _passheads()
    # the keyed upsert: Task_has_Rank is derivation-owned AND key-spanned
    assert ("keyed", "Task_has_Rank") in got
    # the acyclic sweep: Task_has_Cost derives from Peer facts, no cycle
    assert ("sweep", "Task_has_Cost") in got
    # the DRed pass: Task_is_reachable supports itself through its own head
    assert ("dred", "Task_is_reachable") in got


def test_asserted_fact_types_never_join_the_table():
    got = _passheads()
    heads = {h for (_p, h) in got}
    assert "Task_blocks_Task" not in heads       # asserted m:n, machine-free
    assert "Peer_has_Cost" not in heads          # asserted, rule INPUT only
    assert "Peer_serves_Task" not in heads


def test_unmarked_ruled_heads_stay_out_of_the_destructive_passes():
    # 'Task is urgent.' carries NO storage marker: ruled-but-plain, the kind
    # protocol's migration audit calls data (asserted rows would die in a
    # sweep). It joins no destructive pass — exactly run_rules' behavior.
    got = _passheads()
    assert "Task_is_urgent" not in {h for (_p, h) in got}


def test_derived_and_stored_heads_rederive_at_run_rules():
    # the regression this slice fixes: before the OWNED gate, a non-keyed **
    # head sat in NO pass, so its rules never fired after the 0.9.0 swap
    # (the tasks board's frozen recommendations). Now the sweep owns it.
    D, _rep = forml.compile_model(MODEL + INSTANCES)
    D = system.run_rules(D)
    cost = {tuple(r) for r in _pop_rows(D, "Task_has_Cost")}
    assert ("t1", "5") in cost and ("t2", "7") in cost
    reach = {tuple(r) for r in _pop_rows(D, "Task_is_reachable")}
    assert ("t1",) in reach          # t1 blocks t2, and t2 has a Cost
    assert ("t2",) not in reach      # t2 blocks nothing
    rank = {tuple(r) for r in _pop_rows(D, "Task_has_Rank")}
    assert ("t1", "1") in rank       # the keyed pass, unchanged by the slice
