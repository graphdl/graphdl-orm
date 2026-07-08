"""The compile-time machine fold: instance facts of trigger fact types ARE
the event stream when they arrive as readings (the tasks board's
sm-migration class, 2026-07-08), and the write path's incremental fold never
sees them. machine_fold runs the same fold once at compile — per governed
entity, the machine itself orders the greedy walk — so a recompile
re-derives statuses from the readings deterministically."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system


MODEL = """Status is a value type.
Ticket is an entity type.
Ticket has Status.
Each Ticket has at most one Status.
Ticket is closed.
Ticket is reopened.
State Machine Definition 'Flow' is for Noun 'Ticket'.
Status 'open' is initial in State Machine Definition 'Flow'.
Transition 'close' is from Status 'open'.
Transition 'close' is to Status 'done'.
Transition 'close' is triggered by Fact Type 'Ticket is closed'.
Transition 'reopen' is from Status 'done'.
Transition 'reopen' is to Status 'open'.
Transition 'reopen' is triggered by Fact Type 'Ticket is reopened'.
"""


def _fold(model):
    D, _ = forml.compile_model(model)
    D = system.run_rules(D)
    D = system.status_facts(D)
    return system.machine_fold(D)


def _statuses(D):
    return dict(system._status_rows(D, "Ticket"))


def test_a_readings_event_folds_from_initial():
    D = _fold(MODEL + "Ticket 't1' is closed.\n")
    assert _statuses(D).get("t1") == "done"


def test_the_machine_orders_the_walk():
    # close then reopen is the only chain that fires both events; the
    # fold finds it regardless of statement order in the readings
    D = _fold(MODEL
              + "Ticket 't2' is reopened.\n"
              + "Ticket 't2' is closed.\n")
    assert _statuses(D).get("t2") == "open"


def test_an_unfireable_event_no_ops_but_init_covers_the_player():
    # reopen from initial 'open' cannot fire (write-path no-op
    # semantics) — the event row remains a fact, and SM init still
    # materializes the initial for its player (an entity is an entity
    # by playing a fact)
    D = _fold(MODEL + "Ticket 't3' is reopened.\n")
    assert _statuses(D).get("t3") == "open"
    assert ("t3",) in {tuple(r) for r in
                       system._pop_rows(D, "Ticket_is_reopened")}


def test_no_events_no_fold():
    D = _fold(MODEL)
    assert _statuses(D) == {}


import pytest


@pytest.mark.xfail(reason='the specimen is consumed by an earlier '
                   'recognizer before classification — it vanishes '
                   'entirely (neither unparsed nor prose nor a fact '
                   'type); the prose-path guard below it is in, the '
                   'recognizer-path fix is the board p3',
                   strict=True)
def test_a_malformed_machine_statement_reports_loudly():
    # the arrow-glue-loud class: "Status 'X' is initial." (machine
    # clause missing) must land in unparsed, never silently in prose —
    # the support board lost its Feature Request initial to this
    line = "Status 'open' is initial." + chr(10)
    D, rep = forml.compile_model(MODEL + line)
    assert any("is initial" in u for u in rep["unparsed"])
