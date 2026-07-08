"""The desktop container (Samuel 2026-07-08): controls are DEFS
(register_form 'control:<role>' per toolkit, the MonoCross
Register/Resolve), and control is inverted via bind — a control's
event applies the trigger fact type's own function (>>= into the
write path); the store is the state monad and the window re-renders
from D'. The trees are the canon's (system:view_list / view_detail /
view_menu); this suite drives the container without pixels and the tk
window headlessly when a display exists."""
import os

import pytest

import pyarest.prims  # noqa: F401
from pyarest import apps, showui
from pyarest.kernel import resolve_form


MODEL = """Status is a value type.
Note is a value type.
Ticket is an entity type.
Ticket has Note.
Each Ticket has at most one Note.
Ticket is closed.
State Machine Definition 'Flow' is for Noun 'Ticket'.
Status 'open' is initial in State Machine Definition 'Flow'.
Transition 'close' is from Status 'open'.
Transition 'close' is to Status 'done'.
Transition 'close' is triggered by Fact Type 'Ticket is closed'.
Ticket 't1' has Note 'fresh'.
"""


def _mk(tmp_path):
    root = str(tmp_path)
    d = os.path.join(root, "flow", "readings")
    os.makedirs(d)
    with open(os.path.join(d, "app.md"), "w", encoding="utf-8") as f:
        f.write(MODEL)
    reg = apps.Registry(root)
    reg.compile("flow")
    return reg


def test_the_canon_trees_carry_the_component_vocabulary():
    lst = showui.view_list_tree([("t1", "first")])
    assert lst[0] == "list" and lst[1][0] == ("item", "t1", "first")
    det = showui.view_detail_tree([("Status", "open"), ("Owner", None)])
    assert det[0] == "detail"
    assert ("field", "Status", "open") in det[1]
    assert ("field", "Owner", "") in det[1]
    menu = showui.view_menu_tree("open", [("open", "close", "done")])
    assert menu == ("menu", (("button", "close", "done"),))


def test_controls_are_defs_registered_per_toolkit():
    # the MonoCross Register/Resolve seam: one constructor per role,
    # resolved through the SAME layered registry the engine's
    # functional forms use
    for role in ("list", "detail", "menu"):
        assert resolve_form("control:" + role, "tk") is not None
    assert resolve_form("control:list", "no-such-toolkit") is None


def test_events_bind_to_the_facts_function(tmp_path):
    # the inversion of control: firing the menu's event applies the
    # trigger fact type through the write path (>>=), and the machine
    # advances — no app code between the control and the fact
    reg = _mk(tmp_path)
    c = showui.Container(reg, "flow")
    assert c.nouns() == ["Ticket"]
    assert "t1" in c.entities("Ticket")
    acts = c.actions("Ticket", "t1")
    assert acts["status"] == "open"
    assert acts["actions"] == [{"event": "Ticket_is_closed", "to": "done"}]
    receipt = c.fire("Ticket_is_closed", "t1")
    assert receipt["committed"] is True
    after = c.actions("Ticket", "t1")
    assert after["status"] == "done" and after["actions"] == []


def test_the_entry_tree_is_canon_classified(tmp_path):
    # system:view_entry over system:ev_cols — the fact type IS the
    # input's SubmitKey; unary columns carry their kind for the
    # checkbox realization
    reg = _mk(tmp_path)
    c = showui.Container(reg, "flow")
    tree = c.entry_tree("Ticket")
    assert tree[0] == "entry"
    inputs = {n[1]: (n[2], n[3]) for n in tree[1]
              if isinstance(n, tuple) and n[0] == "input"}
    assert inputs["Ticket_has_Note"] == ("note", "value")
    assert inputs["Ticket_is_closed"][1] == "unary"


def test_the_entry_submit_binds_each_input_to_its_fact(tmp_path):
    # the SubmitKey pattern inverted: submit = one >>= per filled
    # input into its OWN fact type's apply; the created entity is
    # immediately live with its machine initial (SM init at the
    # write path arrives later; the fold covers it at compile)
    reg = _mk(tmp_path)
    c = showui.Container(reg, "flow")
    receipts = c.create("Ticket", "t9", [
        ("Ticket_has_Note", "value", "fresh"),
        ("Ticket_is_closed", "unary", False),
    ])
    assert [r["committed"] for r in receipts] == [True]
    assert "t9" in c.entities("Ticket")
    got = c.entity("Ticket", "t9")
    assert got["fields"].get("Note") == "fresh"


def test_the_pane_stack_assigns_and_clears(tmp_path):
    # the survey's PaneManager rules: list -> Master, detail -> Detail,
    # entry -> Popover, tabs -> Tabs; a master navigation clears the
    # detail and popover histories (the split-view rule); back pops
    # per pane
    reg = _mk(tmp_path)
    c = showui.Container(reg, "flow")
    assert c.navigate(("tabs", "Ticket")) == "tabs"
    assert c.navigate(("list", "Ticket")) == "master"
    assert c.navigate(("detail", "Ticket", "t1")) == "detail"
    assert c.navigate(("entry", "Ticket")) == "popover"
    assert c.stacks["detail"].current == ("detail", "Ticket", "t1")
    # a new master context invalidates detail + popover
    c.navigate(("list", "Ticket"))
    assert c.stacks["detail"].current is None
    assert c.stacks["popover"].current is None
    # per-pane back
    c.navigate(("detail", "Ticket", "t1"))
    c.navigate(("detail", "Ticket", "t2"))
    assert c.back("detail") == ("detail", "Ticket", "t1")
    assert c.back("detail") is None
    # the flattened view state walks pane ordinal order
    assert c.stack[0][0] == "tabs" and c.stack[-1][0] == "list"


def test_the_window_renders_headlessly(tmp_path):
    try:
        import tkinter as tk
        probe = tk.Tk()
        probe.destroy()
    except Exception:
        pytest.skip("no display / tkinter unavailable")
    reg = _mk(tmp_path)
    root, c = showui.show(reg, "flow", mainloop=False)
    try:
        root.update_idletasks()
        assert c.stack and c.stack[-1] == ("list", "Ticket")
    finally:
        root.destroy()
