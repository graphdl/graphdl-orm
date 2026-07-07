"""select_component on the new engine (the UI redirect, 2026-07-08:
binding doctrine — AREST.tex §Platform binding — so the Component
registry is ORDINARY FACTS in a registry app and selection is a scoring
VERB on the one table; toolkit implementations register in DEFS per the
iFactr pattern, outside this verb's concern). Contract mirrors the
legacy tool: intent + constraints in, ranked {component, role, toolkit,
symbol, score} records out — intent matched case-insensitively as a
substring of the Component Role."""
import pyarest.prims  # noqa: F401
from pyarest import apps, protocol

REGISTRY = """Component(.id) is an entity type.
Component Role is a value type.
Toolkit is a value type.
Symbol is a value type.
Trait is a value type.
Component has Component Role.
Component is implemented by Symbol in Toolkit.
Component has Trait.

Component 'date-basic' has Component Role 'date picker'.
Component 'date-basic' is implemented by Symbol 'DatePicker' in Toolkit 'monoview'.
Component 'date-touch' has Component Role 'date picker'.
Component 'date-touch' is implemented by Symbol 'TouchDate' in Toolkit 'ifactr'.
Component 'date-touch' has Trait 'touch'.
Component 'grid-main' has Component Role 'data grid'.
Component 'grid-main' is implemented by Symbol 'Grid' in Toolkit 'monoview'.
"""


def _mk(tmp_path):
    d = tmp_path / "_components" / "readings"
    d.mkdir(parents=True)
    (d / "app.md").write_text(REGISTRY, encoding="utf-8")
    reg = apps.Registry(str(tmp_path))
    reg.compile("_components")
    return reg


def test_intent_matches_role_substring_and_ranks(tmp_path):
    reg = _mk(tmp_path)
    out = protocol.select_component(reg, "I need a date picker",
                                    app="_components")
    picks = out["components"]
    assert len(picks) == 2
    assert {p["component"] for p in picks} == {"date-basic", "date-touch"}
    assert all(p["role"] == "date picker" for p in picks)
    assert all(p["toolkit"] and p["symbol"] for p in picks)
    assert picks == sorted(picks, key=lambda p: -p["score"])


def test_trait_constraint_prefers_the_matching_component(tmp_path):
    reg = _mk(tmp_path)
    out = protocol.select_component(reg, "date picker", traits=["touch"],
                                    app="_components")
    picks = out["components"]
    assert picks[0]["component"] == "date-touch", \
        "the trait-satisfying component must outrank the rest"


def test_no_match_answers_empty_not_error(tmp_path):
    reg = _mk(tmp_path)
    out = protocol.select_component(reg, "a kalman filter",
                                    app="_components")
    assert out["components"] == []


def test_the_verb_rides_the_one_table(tmp_path):
    reg = _mk(tmp_path)
    assert "select_component" in protocol.verbs()
    out = protocol._dispatch(reg, "select_component",
                             {"intent": "data grid", "app": "_components"})
    assert out["components"][0]["component"] == "grid-main"
