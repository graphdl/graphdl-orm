"""The apps protocol (the swap contract, part 1): an app is a directory carrying
readings/ and a per-app .db — the same layout the old engine serves at
Repos/apps — and the engine holds one active app at a time with a persistent
marker. In substrate terms an app is a STORE: its readings compile through the
same create to the lfp, its .db is the sqlite snapshot of that store, and
recompile is a FROM-SCRATCH rebuild by design (fix-not-inherit: the old engine's
incremental compile left superseded projected rows, its own ledger prescribes
delete-and-rebuild, so pyarest makes the rebuild the semantics and lets frozen
ingestion make it cheap)."""
import os

import pyarest.prims  # noqa: F401
from pyarest import apps


def _mk_app(root, name, readings):
    d = os.path.join(root, name, "readings")
    os.makedirs(d)
    for fn, text in readings.items():
        with open(os.path.join(d, fn), "w", encoding="utf-8") as f:
            f.write(text)


TASKS = {"core.md": """Task(.id) is an entity type.
Person(.name) is an entity type.
Status is a value type.
Task has Status.
Person owns Task.
Task 't1' has Status 'open'.
Person 'sam' owns Task 't1'.
"""}


def test_list_use_compile_and_query(tmp_path):
    root = str(tmp_path)
    _mk_app(root, "tasks", TASKS)
    reg = apps.Registry(root)
    assert [a["name"] for a in reg.list()] == ["tasks"]
    reg.use("tasks")
    assert reg.current() == "tasks"
    rep = reg.compile("tasks")
    assert rep["unparsed"] == []
    assert os.path.exists(os.path.join(root, "tasks", "tasks.db"))
    assert set(reg.query("tasks", "Task_has_Status")) == {("t1", "open")}
    assert set(reg.query("tasks", "Person_owns_Task")) == {("sam", "t1")}


def test_the_active_app_marker_persists(tmp_path):
    root = str(tmp_path)
    _mk_app(root, "a", TASKS)
    _mk_app(root, "b", TASKS)
    apps.Registry(root).use("b")
    assert apps.Registry(root).current() == "b"                # a NEW registry reads it


def test_recompile_supersedes_removed_facts(tmp_path):
    # the fix-not-inherit gate: deleting a reading's instance fact must remove it
    # from the compiled population, no stale rows, no delete-the-db ritual
    root = str(tmp_path)
    _mk_app(root, "t", TASKS)
    reg = apps.Registry(root)
    reg.compile("t")
    assert set(reg.query("t", "Task_has_Status")) == {("t1", "open")}
    p = os.path.join(root, "t", "readings", "core.md")
    text = open(p, encoding="utf-8").read().replace(
        "Task 't1' has Status 'open'.\n", "")
    open(p, "w", encoding="utf-8").write(text)
    reg.compile("t")
    assert set(reg.query("t", "Task_has_Status")) == set()     # superseded, gone


def test_query_reads_the_snapshot_without_a_compile(tmp_path):
    root = str(tmp_path)
    _mk_app(root, "t", TASKS)
    apps.Registry(root).compile("t")
    reg2 = apps.Registry(root)                                 # fresh process
    assert set(reg2.query("t", "Person_owns_Task")) == {("sam", "t1")}


def test_sql_over_the_app(tmp_path):
    root = str(tmp_path)
    _mk_app(root, "t", TASKS)
    reg = apps.Registry(root)
    reg.compile("t")
    # symbols by default (2026-07-08): cells reference the symbols table;
    # atom text is reached by the join, which keeps the db inspectable
    rows = reg.sql("t", "SELECT name, contents FROM cells WHERE name LIKE '%Task_has_Status%'")
    assert rows and rows[0][1]
    syms = reg.sql("t", "SELECT text FROM symbols")
    assert any("t1" in t for (t,) in syms)


def _mk_pkg(root, name, deps):
    import json
    with open(os.path.join(root, name, "package.json"), "w",
              encoding="utf-8") as f:
        json.dump({"name": f"arest-{name}", "dependencies": deps}, f)


def test_package_dependency_readings_compile_leaf_first(tmp_path):
    # the TS engine's dependencyClosure semantics (apps.ts): file: deps'
    # readings enter the compile BEFORE the app's own, so the dependency's
    # nouns are in context when the app's facts arrive
    root = str(tmp_path)
    _mk_app(root, "lib", {"app.md": """Widget(.id) is an entity type.
Grade is a value type.
Widget has Grade.
Widget 'w1' has Grade '5'.
"""})
    _mk_app(root, "top", {"app.md": "Widget 'w2' has Grade '7'.\n"})
    _mk_pkg(root, "top", {"arest-lib": "file:../lib"})
    reg = apps.Registry(root)
    order = reg._readings("top")
    assert [os.path.basename(os.path.dirname(os.path.dirname(p)))
            for p in order] == ["lib", "top"]
    reg.compile("top")
    assert set(reg.query("top", "Widget_has_Grade")) == {("w1", 5), ("w2", 7)}


def test_dependency_diamond_reads_once_and_cycles_stop(tmp_path):
    # diamond: top -> l1, l2 -> base (base read ONCE, before both libs);
    # cycle: l1 -> l2 -> l1 terminates at the visited set
    root = str(tmp_path)
    _mk_app(root, "base", {"app.md": "Thing(.id) is an entity type.\n"})
    _mk_app(root, "l1", {"app.md": "Thing 'a' is an entity type.\n"})
    _mk_app(root, "l2", {"app.md": "Thing 'b' is an entity type.\n"})
    _mk_app(root, "top", {"app.md": "Thing 'c' is an entity type.\n"})
    _mk_pkg(root, "l1", {"arest-base": "file:../base", "arest-l2": "file:../l2"})
    _mk_pkg(root, "l2", {"arest-base": "file:../base", "arest-l1": "file:../l1"})
    _mk_pkg(root, "top", {"arest-l1": "file:../l1", "arest-l2": "file:../l2"})
    reg = apps.Registry(root)
    order = [os.path.basename(os.path.dirname(os.path.dirname(p)))
             for p in reg._readings("top")]
    assert order.count("base") == 1                     # diamond: once
    assert order.index("base") < order.index("l1")      # leaf-first
    assert order[-1] == "top"                           # the app itself last
