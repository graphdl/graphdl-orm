"""The generator family (punchlist entry 8), starting with dsl: the per-noun
model summary the old engine persists as dsl:<Noun> cells (noun, object
type, the reading texts, the verbalized constraints as kind-text pairs, the
machine transitions). Generated at compile beside the layout cells, so the
claude cutover carries its generator complement forward."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system


MODEL = """Status is a value type.
Ticket is an entity type.
Ticket has Status.
Each Ticket has at most one Status.
State Machine Definition 'Flow' is for Noun 'Ticket'.
Status 'open' is initial in State Machine Definition 'Flow'.
Transition 'close' is from Status 'open'.
Transition 'close' is to Status 'done'.
Transition 'close' is triggered by Fact Type 'close'.
"""


def test_dsl_cells_generate_per_noun():
    D, rep = forml.compile_model(MODEL)
    D = system.run_rules(D)
    D = system.generator_cells(D)
    rows = system._pop_rows(D, "dsl:Ticket")
    assert len(rows) == 1
    row = rows[0]
    got = dict(zip(("noun", "object_type", "readings", "constraints",
                    "transitions"), row))
    assert got["noun"] == "Ticket"
    assert got["object_type"] == "entity"
    assert "Ticket has Status" in got["readings"]
    assert any(k == "UC" and "at most one Status" in text
               for (k, text) in got["constraints"])
    assert ("close", "open", "done") in got["transitions"]
    # the value type gets its own cell with its kind
    vrows = system._pop_rows(D, "dsl:Status")
    assert vrows and vrows[0][1] == "value"
