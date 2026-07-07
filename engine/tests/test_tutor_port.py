"""The tutor rides the new engine (ports the legacy WASM entry): the
sandbox IS an app — _tutor's readings are copies of tutor/domains, reset
== recompile — and the tutor_* verbs are the first-class verbs scoped to
it plus the lesson reader and the expect checker (_format.md's grammar:
query/list contains|count, get equals, status is, violations include).
Hermetic here: AREST_TUTOR_DIR points at a tmp fixture; the REAL
tutor/domains corpus was probe-compiled clean separately (ledger,
2026-07-08)."""
import pyarest.prims  # noqa: F401
from pyarest import apps, protocol

DOMAIN = """Widget(.id) is an entity type.
Color is a value type.
Widget has Color.
Each Widget has at most one Color.

Authoring Step(.id) is an entity type.
Authoring Step Order is a value type.
Authoring Guidance is a value type.
Authoring Tool is a value type.
Authoring Step has Authoring Step Order.
Authoring Step has Authoring Guidance.
Authoring Step recommends Authoring Tool.

Authoring Step 'declare' has Authoring Step Order '1'.
Authoring Step 'declare' has Authoring Guidance 'name the noun first'.
Authoring Step 'declare' recommends Authoring Tool 'propose'.
Authoring Step 'populate' has Authoring Step Order '2'.
Authoring Step 'populate' has Authoring Guidance 'assert instance facts'.
"""

LESSON = """# Lesson easy.1: MAKE A WIDGET

**Goal:** a Widget exists with Color red
**Prereqs:** none

Make one widget the conversational way.

## Do it

~~~ apply
{"fact_type": "Widget_has_Color", "fact": ["w1", "red"]}
~~~

## Check

~~~ expect
query Widget_has_Color contains {"Widget": "w1", "Color": "red"}
~~~

**Next:** none
"""


def _mk(tmp_path, monkeypatch):
    root = tmp_path / "tutorroot"
    (root / "domains").mkdir(parents=True)
    (root / "lessons" / "easy").mkdir(parents=True)
    (root / "domains" / "app.md").write_text(DOMAIN, encoding="utf-8")
    (root / "lessons" / "easy" / "01-make-a-widget.md").write_text(
        LESSON, encoding="utf-8")
    monkeypatch.setenv("AREST_TUTOR_DIR", str(root))
    appsdir = tmp_path / "apps"
    appsdir.mkdir()
    return apps.Registry(str(appsdir))


def test_reset_bootstraps_the_sandbox_as_an_app(tmp_path, monkeypatch):
    reg = _mk(tmp_path, monkeypatch)
    out = protocol.tutor_reset(reg)
    assert out["app"] == "_tutor" and out["reset"] is True
    assert any(x["name"] == "_tutor" for x in reg.list())
    # reset again: idempotent rebootstrap
    assert protocol.tutor_reset(reg)["reset"] is True


def test_list_and_get_parse_the_lessons(tmp_path, monkeypatch):
    _mk(tmp_path, monkeypatch)
    lessons = protocol.tutor_list()["lessons"]
    assert lessons == [{"lesson": "easy/01", "track": "easy",
                        "title": "MAKE A WIDGET",
                        "goal": "a Widget exists with Color red"}]
    got = protocol.tutor_get("easy/01")
    assert got["title"] == "MAKE A WIDGET"
    assert got["fences"] == [{"tag": "apply", "body":
                              '{"fact_type": "Widget_has_Color", '
                              '"fact": ["w1", "red"]}'}]
    assert got["expect"].startswith("query Widget_has_Color contains")


def test_the_check_flips_when_the_learner_does_the_lesson(tmp_path,
                                                          monkeypatch):
    reg = _mk(tmp_path, monkeypatch)
    protocol.tutor_reset(reg)
    before = protocol.tutor_check(reg, "easy/01")
    assert before["passed"] is False
    fence = protocol.tutor_get("easy/01")["fences"][0]
    import json
    args = json.loads(fence["body"])
    receipt = protocol._dispatch(reg, "tutor_apply", args)
    assert receipt["committed"] is True
    after = protocol.tutor_check(reg, "easy/01")
    assert after["passed"] is True


def test_authoring_joins_the_workflow_steps_in_order(tmp_path, monkeypatch):
    reg = _mk(tmp_path, monkeypatch)
    protocol.tutor_reset(reg)
    out = protocol.tutor_authoring(reg)
    steps = out["steps"]
    assert [s["step"] for s in steps] == ["declare", "populate"]
    assert steps[0]["order"] == 1
    assert steps[0]["guidance"] == "name the noun first"
    assert steps[0]["tools"] == ["propose"]
    assert steps[1]["tools"] == []


def test_the_tutor_verbs_ride_the_one_table(tmp_path, monkeypatch):
    reg = _mk(tmp_path, monkeypatch)
    for v in ("tutor_list", "tutor_get", "tutor_check", "tutor_reset",
              "tutor_apply", "tutor_query", "tutor_compile",
              "tutor_propose", "tutor_actions"):
        assert v in protocol.verbs()
    protocol._dispatch(reg, "tutor_reset", {})
    rows = protocol._dispatch(reg, "tutor_query",
                              {"fact_type": "Widget_has_Color"})
    assert rows["app"] == "_tutor" and rows["rows"] == []
