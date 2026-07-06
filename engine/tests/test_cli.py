"""The one-shot CLI, the resident's write delegate: cli.py at the repo root
self-registers the package (the conftest bootstrap, so no install and no cwd
assumption) and runs exactly one Registry verb per invocation, answering one
JSON receipt on stdout. The Rust resident spawns it for apply, retract, and
apps_compile, then reloads the app's sidecar, so the write path stays the one
Python pipeline and the read path stays hot in Rust."""
import json
import os
import subprocess
import sys

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_CLI = os.path.join(_ROOT, "cli.py")

MODEL = """Status is a value type.
Ticket is an entity type.
Ticket has Status.
Each Ticket has at most one Status.
"""


def _mk(tmp_path):
    base = tmp_path / "apps"
    (base / "flow" / "readings").mkdir(parents=True)
    (base / "flow" / "readings" / "app.md").write_text(MODEL, encoding="utf-8")
    return str(base)


def _run(*args):
    return subprocess.run([sys.executable, _CLI] + list(args),
                          capture_output=True, text=True, cwd=_ROOT)


def test_cli_compile_apply_and_the_sidecar_refresh(tmp_path):
    base = _mk(tmp_path)
    out = _run("compile", "--apps-dir", base, "flow")
    assert out.returncode == 0, out.stderr[-500:]
    rep = json.loads(out.stdout)
    assert rep["app"] == "flow" and rep["unparsed"] == []
    side = os.path.join(base, "flow", "flow.store.json")
    with open(side, encoding="utf-8") as f:
        before = json.load(f)["d"]
    out = _run("apply", "--apps-dir", base, "flow",
               "Ticket_has_Status", json.dumps(["t1", "open"]))
    assert out.returncode == 0, out.stderr[-500:]
    receipt = json.loads(out.stdout)
    assert receipt["committed"] is True and receipt["fact"] == ["t1", "open"]
    with open(side, encoding="utf-8") as f:
        after = json.load(f)["d"]
    assert after != before


def test_cli_refusal_answers_the_receipt_and_exit_one(tmp_path):
    # a second value on the functional fact type refuses (the UC made
    # structural); the receipt still answers on stdout, the exit code says 1
    base = _mk(tmp_path)
    assert _run("compile", "--apps-dir", base, "flow").returncode == 0
    assert _run("apply", "--apps-dir", base, "flow",
                "Ticket_has_Status", '["t1", "open"]').returncode == 0
    out = _run("apply", "--apps-dir", base, "flow",
               "Ticket_has_Status", '["t1", "closed"]')
    assert out.returncode == 1
    receipt = json.loads(out.stdout)
    assert receipt["committed"] is False


def test_cli_read_verbs_answer_json(tmp_path):
    # the delegation long tail: every Registry read verb the resident does
    # not serve natively rides the same one-shot path, so the daily driver
    # is complete by delegation first and canonical as demanded later
    base = _mk(tmp_path)
    assert _run("compile", "--apps-dir", base, "flow").returncode == 0
    assert _run("apply", "--apps-dir", base, "flow",
                "Ticket_has_Status", '["t1", "open"]').returncode == 0
    out = _run("get", "--apps-dir", base, "flow", "Ticket", "t1")
    assert out.returncode == 0, out.stderr[-500:]
    got = json.loads(out.stdout)
    assert got["id"] == "t1" and got["noun"] == "Ticket"
    out = _run("schema", "--apps-dir", base, "flow")
    assert out.returncode == 0
    sch = json.loads(out.stdout)
    assert any("Ticket_has_Status" in json.dumps(x) for x in sch.values())
    out = _run("sql", "--apps-dir", base, "flow",
               "SELECT COUNT(*) AS n FROM sqlite_master")
    assert out.returncode == 0
    assert json.loads(out.stdout)[0][0] >= 1
    out = _run("validate", "--apps-dir", base, "flow")
    assert out.returncode == 0
    assert json.loads(out.stdout)["app"] == "flow"


def test_cli_sql_user_error_answers_an_error_envelope(tmp_path):
    # a syntactically bad statement is the CALLER'S error, not a crash: the
    # CLI answers {"error": ...} at exit 0 so the resident relays it as a
    # result (the old MCP's envelope behavior), reserving nonzero exits for
    # real crashes
    base = _mk(tmp_path)
    assert _run("compile", "--apps-dir", base, "flow").returncode == 0
    out = _run("sql", "--apps-dir", base, "flow", "SELEC nonsense FROM")
    assert out.returncode == 0, out.stderr[-300:]
    got = json.loads(out.stdout)
    assert "error" in got and "syntax" in got["error"].lower()


def test_cli_unknown_verb_is_a_usage_error(tmp_path):
    out = _run("frobnicate", "--apps-dir", str(tmp_path), "x")
    assert out.returncode == 2
    assert "usage" in out.stderr.lower()
