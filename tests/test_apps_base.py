"""The base preload: apps compile ATOP the vendored base readings (the old
engine folds CORE_READINGS ahead of every app's own — lib.rs metamodel_readings),
so an app's rules resolve base-declared types (Resource, Status, State Machine,
Timestamp) without redeclaring them. The base compiles once per engine
fingerprint through frozen ingestion; app statements fold on top with the known
context (names, subtypes, fact types) READ OFF the thawed store."""
import os

import pyarest.prims  # noqa: F401
from pyarest import apps, forml


BASE = """Status is a value type.
Resource is an entity type.
State Machine is an entity type.
State Machine is for Resource.
State Machine is currently in Status.
"""

APP = """Task is an entity type.
Task Status is a value type.
Task has Task Status.

* Resource is currently in Status iff some State Machine is for that Resource and that State Machine is currently in that Status.

* Task has Task Status iff that Resource is currently in some Status and Task Status is Status and Task is Resource.

State Machine 'sm1' is for Resource 't1'.
State Machine 'sm1' is currently in Status 'in_progress'.
"""


def _mk(tmp_path):
    base_dir = tmp_path / "base"
    base_dir.mkdir()
    (base_dir / "core.md").write_text(BASE, encoding="utf-8")
    apps_dir = tmp_path / "apps"
    (apps_dir / "board" / "readings").mkdir(parents=True)
    (apps_dir / "board" / "readings" / "app.md").write_text(APP, encoding="utf-8")
    return apps.Registry(str(apps_dir), base_dir=str(base_dir),
                         cache_dir=str(tmp_path / "frozen"))


def test_an_app_rule_resolves_base_declared_types(tmp_path):
    reg = _mk(tmp_path)
    rep = reg.compile("board")
    assert rep["unparsed"] == []
    assert rep["rule_diagnostics"] == []
    # the anaphoric rules fire across the base/app seam: base instances derive
    # the app's Task_has_Task_Status through the coercion re-keying
    assert reg.query("board", "Task_has_Task_Status") == [("t1", "in_progress")]


def test_the_base_freezes_once(tmp_path):
    reg = _mk(tmp_path)
    reg.compile("board")
    frozen = [f for f in os.listdir(str(tmp_path / "frozen"))
              if f.startswith("ingest-")]
    assert len(frozen) == 1
    reg.compile("board")                                      # thaw, not re-ingest
    frozen2 = [f for f in os.listdir(str(tmp_path / "frozen"))
               if f.startswith("ingest-")]
    assert frozen2 == frozen


def test_the_vendored_base_is_the_old_engines_backbone():
    # the eight CORE_READINGS files plus the deployed evolution overlay
    # (evolution + csdp — the live tasks db carries Domain Change and Rmap
    # machinery, proving the old binary compiled them in); no compile here
    # (the suite pays the base ingest only through the registry tests' tiny base)
    from pyarest import paths
    d = os.path.join(paths.root(), "shared", "base")
    names = sorted(os.listdir(d))
    assert names == ["core.md", "csdp.md", "evolution.md", "induction.md",
                     "instances.md", "naming.md", "outcomes.md", "security.md",
                     "state.md", "validation.md"]
    text = "\n\n".join(open(os.path.join(d, n), encoding="utf-8").read()
                       for n in names)
    assert len(forml.statements(text)) > 900
