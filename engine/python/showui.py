"""The desktop container — the abstract UI's first realization (Samuel,
2026-07-08: "the arest cli should show Windows desktop apps using the
same tricks iFactr does — registering controls in DEFS and inverting
control via bind (like >>=) to the fact's function").

The two tricks, literally:
- CONTROLS ARE DEFS: a toolkit registers one constructor per abstract
  control role through the SAME layered Register/Resolve the engine
  already uses for functional forms (kernel.register_form /
  resolve_form — MonoCross's MXContainer pattern). The element
  vocabulary is the canon view family's (system:view_menu /
  view_detail / view_list), whose node kinds are Component-registry
  Roles — so a node resolves to a toolkit implementation exactly the
  way select_component resolves widgets.
- INVERSION OF CONTROL VIA BIND: a control's native event never calls
  app code; it is BOUND to the fact's own function — the apply path
  for the trigger fact type the schema derives (actions answers
  ⟨event-ft, to⟩ pairs). The store is the state monad: event >>=
  apply(create:<ft>) answers the receipt and the container re-renders
  from D'. Facts render; events apply; refusals surface the
  violations verbatim (Corollary 1 — the reading is the explanation).

The container itself is MonoCross-shaped: a navigation STACK of
(noun, id) perspectives — list is the noun's perspective, detail the
entity's — with back = pop. The tk toolkit ships in-module because
tkinter is stdlib (zero-dep discipline); Slint/WPF/react register the
same control:* slots elsewhere."""

from .kernel import register_form, resolve_form


# ---- the view trees: THE CANON's, this module only prepares operands ----
def view_list_tree(items):
    """system:view_list over ⟨id, label⟩ pairs → ⟨list, ⟨⟨item,id,label⟩…⟩⟩."""
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam, atom as A
    return from_lam(_ap(A("system:view_list"),
                        to_lam(tuple((i, l) for (i, l) in items))))


def view_detail_tree(fields):
    """system:view_detail over ⟨name, value⟩ pairs → ⟨detail, ⟨⟨field,n,v⟩…⟩⟩."""
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam, atom as A
    return from_lam(_ap(A("system:view_detail"),
                        to_lam(tuple((n, "" if v is None else v)
                                     for (n, v) in fields))))


def view_menu_tree(status, triples):
    """system:view_menu over ⟨status, sm-triples⟩ → ⟨menu, ⟨⟨button,ev,to⟩…⟩⟩."""
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam, atom as A
    import pyarest.lam as L
    operand = L.SEQ(L.CONS(A(status))(
        L.CONS(to_lam(tuple(tuple(t) for t in triples)))(L.NIL)))
    return from_lam(_ap(A("system:view_menu"), operand))


def view_entry_tree(D, noun):
    """system:view_entry over THE CANON's classification (system:ev_cols
    ⟨noun, D⟩) → ⟨entry, ⟨⟨input, ft, name, kind⟩…⟩⟩ — the fact type IS
    the input's SubmitKey."""
    from .reduce import apply as _ap
    from .lam import from_lam, atom as A
    import pyarest.lam as L
    pair = L.SEQ(L.CONS(A(noun))(L.CONS(D)(L.NIL)))
    cols = _ap(A("system:ev_cols"), pair)
    return from_lam(_ap(A("system:view_entry"), cols))


# ---- the pane-addressed view stack (iFactr's, survey §7) ----
PANES = ("tabs", "master", "detail", "popover")   # ordinal order load-bearing

# pane choice: the frame KIND is the layer type; the survey's priority
# (attribute > layer type > Detail default) collapses here to the kind
# map — a "list" is the master layer, an entity view the detail, an
# entry form the popover (modal), the noun set the tabs
_PANE_FOR = {"tabs": "tabs", "list": "master",
             "detail": "detail", "entry": "popover"}


class HistoryStack:
    """IHistoryStack's surface: push/pop/pop-to/replace over one pane."""

    def __init__(self):
        self.views = []

    def push(self, frame):
        self.views.append(frame)
        return frame

    def pop(self):
        return self.views.pop() if self.views else None

    def pop_to_root(self):
        del self.views[1:]
        return self.views[0] if self.views else None

    def pop_to(self, frame):
        """PopToView: unwind until `frame` is current (a no-op when it
        is not on the stack — iFactr throws; the container prefers
        grace)."""
        while self.views and self.views[-1] != frame:
            self.views.pop()
        return self.current

    def replace(self, old, new):
        """ReplaceView: swap in place, history above untouched."""
        for i, f in enumerate(self.views):
            if f == old:
                self.views[i] = new
                return new
        return None

    @property
    def current(self):
        return self.views[-1] if self.views else None


class Container:
    """MXContainer's shape over a Registry app: Navigate assigns the
    frame its pane (PaneManager.DisplayView's rule), pushes onto that
    pane's history stack, Back pops per pane, and every frame renders
    by resolving control:* constructors for the current toolkit."""

    def __init__(self, registry, app, toolkit="tk"):
        self.reg = registry
        self.app = app
        self.toolkit = toolkit
        self.active_tab = 0                 # AppNavigationContext.ActiveTab
        self._stacks = {}                   # PaneManager's registry,
        self.gates = []                     # ShouldNavigateFrom pollees

    # -- model reads (facts render) --
    def nouns(self):
        return [n["name"]
                for n in self.reg.schema(self.app)["object_types"]
                if n["kind"] == "ObjectType"]

    def entities(self, noun):
        """The noun's population — Registry.entities is the one home
        (the WPF adapter reads the same verb through the cli)."""
        return self.reg.entities(self.app, noun)

    def items(self, noun):
        """The list rows — Registry.items is the one home."""
        return [tuple(r) for r in self.reg.items(self.app, noun)]

    def entity(self, noun, id):
        return self.reg.get(self.app, noun, id)

    def actions(self, noun, id):
        return self.reg.actions(self.app, noun, id)

    # -- the bind (events apply): event >>= the fact's function --
    def fire(self, event_ft, id):
        """The inverted control: the control's event applies the trigger
        fact type's own function through the write path; the receipt
        (committed, violations) is the whole answer."""
        return self.reg.apply(self.app, event_ft, [id])

    def create(self, noun, id, values):
        """The entry view's submit: each filled input applies ITS OWN fact
        type over ⟨id, value⟩ (unary inputs over ⟨id⟩ when truthy) — one
        >>= per SubmitKey, in classified order, stopping at the first
        refusal. Answers the receipts."""
        receipts = []
        for ft, kind, value in values:
            if kind == "unary":
                if not value:
                    continue
                row = [id]
            else:
                if value in (None, ""):
                    continue
                row = [id, value]
            r = self.reg.apply(self.app, ft, row)
            receipts.append(r)
            if not r.get("committed"):
                break
        return receipts

    def entry_tree(self, noun):
        return view_entry_tree(self.reg._load(self.app), noun)

    # -- navigation: pane assignment + per-pane-per-tab history --
    def stack_for(self, pane, tab=None):
        """PaneManager.FromNavContext: stacks are keyed by ⟨pane, tab⟩
        (the survey's pane + tab*256) — each tab carries its own
        master/detail/popover histories; the tabs pane itself is
        tab-independent."""
        key = (pane, 0 if pane == "tabs"
               else (self.active_tab if tab is None else tab))
        if key not in self._stacks:
            self._stacks[key] = HistoryStack()
        return self._stacks[key]

    @property
    def stacks(self):
        """The ACTIVE tab's stacks, one per pane (the older surface;
        tests and renderers address panes of the current context)."""
        return {p: self.stack_for(p) for p in PANES}

    def pane_for(self, frame, pane=None):
        """GetPreferredPane's priority collapsed to this container's
        channels: an explicit override (OutputOnPane / the frame's own
        pane= request) wins, then the kind map, then Detail — and a
        Tabs target coerces to Master (iApp.Navigate's forcing: content
        never lands ON the tab strip)."""
        chosen = pane or _PANE_FOR.get(frame[0] if frame else None,
                                       "detail")
        if chosen == "tabs" and frame and frame[0] != "tabs":
            chosen = "master"
        return chosen

    def should_navigate(self, frame):
        """The ShouldNavigate gate: every registered pollee (a view's
        ShouldNavigateFrom) may veto by answering False."""
        return all(g(frame) for g in self.gates)

    def navigate(self, frame, clear_history=False, pane=None):
        """DisplayView: gate, assign the pane (override channel first),
        optionally clear its history (RequestType.ClearPaneHistory),
        push. A master navigation clears the DETAIL and POPOVER
        histories of ITS TAB — the split-view rule: a new master
        context invalidates the old detail. Answers the pane, or None
        when a gate vetoed."""
        if not self.should_navigate(frame):
            return None
        pane = self.pane_for(frame, pane)
        if frame and frame[0] == "tabs" and len(frame) > 2:
            self.active_tab = frame[2]
        stack = self.stack_for(pane)
        if clear_history:
            stack.views.clear()
        # iApp.Navigate's dedup: a view already ON the stack is popped
        # TO, never pushed twice — back-behavior stays truthful; the
        # split-view clearing below still applies (a re-navigated
        # master invalidates its detail like any master navigation)
        if frame in stack.views:
            stack.pop_to(frame)
        else:
            stack.push(frame)
        if pane == "master":
            self.stack_for("detail").views.clear()
            self.stack_for("popover").views.clear()
        return pane

    def back(self, pane="detail"):
        """Pop the pane's stack; answer the frame now current there
        (None when the pane emptied — the popover's close)."""
        self.stack_for(pane).pop()
        return self.stack_for(pane).current

    def detail_link(self, frame):
        """iLayer.DetailLink's auto-load: a master frame carrying a
        5th element ⟨…, detail-frame⟩ names the detail its split view
        should open with; answers it (the renderer navigates)."""
        return frame[4] if frame and len(frame) > 4 else None

    @property
    def stack(self):
        """The flattened view state of the ACTIVE tab in pane ordinal
        order (tabs first), topmost frame per occupied pane —
        TopmostPane's shape."""
        return [s.current for p, s in
                ((p, self.stack_for(p)) for p in PANES) if s.current]

    # -- rendering: resolve control constructors through DEFS --
    def render(self, parent, tree, ctx):
        kind = tree[0] if isinstance(tree, tuple) and tree else None
        ctor = resolve_form("control:" + str(kind), self.toolkit)
        if ctor is None:
            raise LookupError(f"no control:{kind} registered for "
                              f"toolkit {self.toolkit!r}")
        return ctor(parent, tree, ctx)


# ---- the tk toolkit: stdlib, in-module (the zero-dep desktop) ----
# The content grids lay out through THE ABSTRACT ENGINE
# (pyarest.uilayout — iFactr's PerformLayout): tk supplies exactly the
# two platform primitives — MEASURE via tkfont metrics and PLACE via
# the place() geometry manager (absolute positioning, the Canvas
# analog). Chrome (menu bars, buttons docked at edges) stays native,
# exactly as iFactr-WPF docks its header/toolbar outside the engine.

def _tk_measure(widget, is_text=False):
    """The tk MEASURE primitive: a closure answering (w, h) under
    constraints from the widget's own font metrics; wrapping text
    reflows (narrower means taller), fixed inputs answer their
    requested size."""
    import tkinter.font as tkfont

    def measure(cw, ch):
        if is_text:
            f = tkfont.Font(font=widget.cget("font"))
            text = widget.cget("text")
            lines = text.split("\n") or [""]
            wide = max((f.measure(t) for t in lines), default=0)
            line_h = f.metrics("linespace")
            if wide <= cw:
                return (min(wide, cw), min(len(lines) * line_h, ch))
            import math
            wrapped = sum(max(1, math.ceil(f.measure(t) / max(cw, 1)))
                          for t in lines)
            return (min(cw, wide), min(wrapped * line_h, ch))
        widget.update_idletasks()
        return (min(widget.winfo_reqwidth(), cw),
                min(widget.winfo_reqheight(), ch))
    return measure


def _tk_place(widget):
    """The tk PLACE primitive: absolute positioning in the host frame;
    wrapping labels get their wraplength pinned to the placed width
    (the platform's realization of the reflowed rectangle)."""
    def place(x, y, w, h):
        try:
            widget.configure(wraplength=max(int(w), 1))
        except Exception:
            pass
        widget.place(x=int(x), y=int(y), width=max(int(w), 1),
                     height=max(int(h), 1))
    return place


def _tk_grid_layout(host, grid, width):
    """Run the abstract engine over the composed grid and size the host
    frame to the answer (the GridView MeasureOverride handoff)."""
    from .uilayout import INF, perform_layout
    w, h = perform_layout(grid, (width, 0), (width, INF))
    host.configure(width=int(w), height=int(h))
    host.pack_propagate(False)
    return w, h


def _tk_list(parent, tree, ctx):
    """The list pane as CONTENT CELLS (the shared ContentCell recipe,
    uicells.content_cell): text star column, the id as subtext, the
    machine status right-aligned as the value — each cell its own
    absolutely-laid frame, clicks bound to the selection."""
    import tkinter as tk
    from .uicells import CELL_HEIGHT, content_cell
    from .uilayout import INF, perform_layout
    frame = tk.Frame(parent)
    canvas = tk.Canvas(frame, width=300, highlightthickness=0)
    bar = tk.Scrollbar(frame, orient="vertical", command=canvas.yview)
    canvas.configure(yscrollcommand=bar.set)
    inner = tk.Frame(canvas)
    canvas.create_window((0, 0), window=inner, anchor="nw")
    y = 0
    width = 300
    items = ctx.get("items") or [
        (str(n[1]), str(n[2]), "") for n in tree[1]
        if isinstance(n, tuple) and len(n) >= 3 and n[0] == "item"]
    for id, text, value in items:
        cell = tk.Frame(inner, width=width, bd=0, relief=tk.FLAT)
        t = tk.Label(cell, text=text, anchor="w",
                     font=("Segoe UI", 10))
        s = tk.Label(cell, text=id, anchor="w", fg="#868686",
                     font=("Segoe UI", 8))
        widgets = [cell, t, s]
        kwargs = {"text": (_tk_measure(t, is_text=True), _tk_place(t)),
                  "subtext": (_tk_measure(s, is_text=True), _tk_place(s))}
        if value:
            v = tk.Label(cell, text=value, fg="#868686",
                         font=("Segoe UI", 9))
            widgets.append(v)
            kwargs["value"] = (_tk_measure(v, is_text=True), _tk_place(v))
        g = content_cell(**kwargs)
        _w, h = perform_layout(g, (width, CELL_HEIGHT), (width, INF))
        cell.configure(height=int(h))
        cell.pack_propagate(False)
        cell.place(x=0, y=y, width=width, height=int(h))
        if ctx.get("on_select"):
            for w in widgets:
                w.bind("<Button-1>",
                       lambda _e, i=id: ctx["on_select"](i))
        y += int(h) + 1
    inner.configure(width=width, height=y)
    canvas.configure(scrollregion=(0, 0, width, y))
    canvas.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
    bar.pack(side=tk.RIGHT, fill=tk.Y)
    frame.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
    return frame


def _tk_detail(parent, tree, ctx):
    """The detail grid THROUGH THE ABSTRACT ENGINE: one auto row per
    field, columns ⟨auto label, star value⟩ — the same composition
    iFactr's GridCell/Instructor builds; values are labels (the reflow
    rule applies), placement is absolute."""
    import tkinter as tk
    from .uilayout import AUTO, NEAR, STAR, Element, Grid, Track
    frame = tk.Frame(parent)
    rows = [n for n in tree[1] if isinstance(n, tuple) and len(n) >= 3
            and n[0] == "field"]
    g = Grid(columns=[Track(AUTO), Track(STAR)],
             rows=[Track(AUTO) for _ in rows],
             padding=(8, 4, 8, 4))
    for i, node in enumerate(rows):
        k = tk.Label(frame, text=str(node[1]) + ":", anchor="ne",
                     justify="right", font=("Segoe UI", 9, "bold"))
        v = tk.Label(frame, text=str(node[2]), anchor="nw",
                     justify="left")
        g.add(Element(_tk_measure(k, is_text=True), _tk_place(k),
                      row=i, col=0, halign=NEAR, valign=NEAR,
                      margin=(0, 2, 6, 2)))
        g.add(Element(_tk_measure(v, is_text=True), _tk_place(v),
                      row=i, col=1, halign=NEAR, valign=NEAR,
                      margin=(0, 2, 0, 2), is_label=True))
    parent.update_idletasks()
    width = max(parent.winfo_width(), 360)
    _tk_grid_layout(frame, g, width)
    frame.pack(side=tk.TOP, fill=tk.BOTH, expand=True)
    return frame


def _tk_menu(parent, tree, ctx):
    import tkinter as tk
    bar = tk.Frame(parent, pady=6)
    for node in tree[1]:
        if isinstance(node, tuple) and len(node) >= 3 and node[0] == "button":
            event_ft, to = str(node[1]), str(node[2])
            # THE BIND: the button's native command IS the fact's function
            tk.Button(bar, text=f"{event_ft} → {to}",
                      command=lambda e=event_ft: ctx["fire"](e)).pack(
                side=tk.LEFT, padx=3)
    bar.pack(side=tk.BOTTOM, fill=tk.X)
    return bar


def _tk_entry(parent, tree, ctx):
    """The entry grid THROUGH THE ABSTRACT ENGINE: ⟨auto label, star
    input⟩ per row, the submit button on its own trailing row — the
    iFactr entry-cell composition, absolutely placed."""
    import tkinter as tk
    from .uilayout import AUTO, NEAR, STAR, STRETCH, Element, Grid, Track
    frame = tk.Frame(parent)
    nodes = [n for n in tree[1] if isinstance(n, tuple) and len(n) >= 4
             and n[0] == "input"]
    g = Grid(columns=[Track(AUTO), Track(STAR)],
             rows=[Track(AUTO) for _ in range(len(nodes) + 2)],
             padding=(10, 6, 10, 6))

    def add_row(i, label, widget, halign=STRETCH):
        k = tk.Label(frame, text=label + ":", anchor="e")
        g.add(Element(_tk_measure(k, is_text=True), _tk_place(k),
                      row=i, col=0, halign=NEAR, valign=NEAR,
                      margin=(0, 3, 6, 3)))
        g.add(Element(_tk_measure(widget), _tk_place(widget),
                      row=i, col=1, halign=halign, valign=NEAR,
                      margin=(0, 3, 0, 3)))

    id_var = tk.StringVar()
    add_row(0, "id", tk.Entry(frame, textvariable=id_var))
    inputs = []                     # (ft, kind, var) — ft IS the SubmitKey
    for i, node in enumerate(nodes, start=1):
        ft, name, kind = str(node[1]), str(node[2]), str(node[3])
        if kind == "unary":
            var = tk.BooleanVar()
            w = tk.Checkbutton(frame, variable=var)
            add_row(i, name, w, halign=NEAR)
        else:
            var = tk.StringVar()
            add_row(i, name, tk.Entry(frame, textvariable=var))
        inputs.append((ft, kind, var))
    btn = tk.Button(frame, text="Create",
                    command=lambda: ctx["submit"](
                        id_var.get(),
                        [(ft, kind, var.get())
                         for ft, kind, var in inputs]))
    g.add(Element(_tk_measure(btn), _tk_place(btn),
                  row=len(nodes) + 1, col=1, halign=NEAR, valign=NEAR,
                  margin=(0, 8, 0, 0)))
    parent.update_idletasks()
    width = max(parent.winfo_width(), 480)
    _tk_grid_layout(frame, g, width)
    frame.pack(side=tk.TOP, fill=tk.BOTH, expand=True)
    return frame


register_form("control:list", _tk_list, "tk")
register_form("control:detail", _tk_detail, "tk")
register_form("control:menu", _tk_menu, "tk")
register_form("control:entry", _tk_entry, "tk")


def show(registry, app, noun=None, mainloop=True):
    """The cli's desktop verb: one window over the app's store, laid out
    by pane — TABS across the top (one per entity noun), MASTER on the
    left (the noun's list), DETAIL on the right (fields + the machine
    menu, every button bound to its trigger fact type's apply), and the
    entry form a POPOVER (a modal Toplevel; its submit binds each
    input's SubmitKey to its fact). Answers (root, container); tests
    drive the container without the loop."""
    import tkinter as tk
    c = Container(registry, app)
    nouns = c.nouns()
    if not nouns:
        raise LookupError(f"app {app!r} has no entity nouns")
    state = {"noun": noun or nouns[0]}
    root = tk.Tk()
    root.title(f"arest — {app}")
    tabs = tk.Frame(root, pady=2)
    tabs.pack(side=tk.TOP, fill=tk.X)
    left = tk.Frame(root)
    left.pack(side=tk.LEFT, fill=tk.BOTH)
    right = tk.Frame(root)
    right.pack(side=tk.RIGHT, fill=tk.BOTH, expand=True)
    status_var = tk.StringVar(value=f"{app} · {state['noun']}")
    tk.Label(root, textvariable=status_var, anchor="w",
             relief=tk.SUNKEN).pack(side=tk.BOTTOM, fill=tk.X)

    def clear(widget):
        for w in widget.winfo_children():
            w.destroy()

    def render_detail(id):
        clear(right)
        noun = state["noun"]
        got = c.entity(noun, id)
        bar = tk.Frame(right, pady=2)
        bar.pack(side=tk.TOP, fill=tk.X)

        def go_back():
            prev = c.back("detail")
            if prev and prev[0] == "detail":
                render_detail(prev[2])
            else:
                clear(right)
        tk.Button(bar, text="◀ Back", command=go_back).pack(side=tk.LEFT,
                                                            padx=3)
        fields = sorted((k, v) for k, v in (got.get("fields") or {}).items()
                        if not isinstance(v, bool))
        c.render(right, view_detail_tree(fields), {})
        acts = c.actions(noun, id)
        triples = [(acts["status"], a["event"], a["to"])
                   for a in acts.get("actions", [])]
        if acts.get("status") is not None:
            menu = view_menu_tree(acts["status"], triples)

            def fire(event_ft):
                receipt = c.fire(event_ft, id)
                status_var.set(
                    ("committed " + event_ft) if receipt.get("committed")
                    else ("REFUSED: " + str(receipt.get("violations"))[:120]))
                render_list()
                render_detail(id)
            c.render(right, menu, {"fire": fire})
        c.navigate(("detail", noun, id))

    def render_entry():
        noun = state["noun"]
        top = tk.Toplevel(root)                       # the popover pane
        top.title(f"New {noun}")
        top.transient(root)

        def submit(id, values):
            if not id:
                status_var.set("REFUSED: an id is required")
                return
            receipts = c.create(noun, id, values)
            bad = [r for r in receipts if not r.get("committed")]
            if bad:
                status_var.set("REFUSED: "
                               + str(bad[0].get("violations"))[:120])
            else:
                status_var.set(f"created {id} ({len(receipts)} facts)")
                top.destroy()
                c.back("popover")
                render_list()
                render_detail(id)
        c.render(top, c.entry_tree(noun), {"submit": submit})
        c.navigate(("entry", noun))

    def render_list():
        clear(left)
        noun = state["noun"]
        items = [(i, i) for i in c.entities(noun)]
        c.render(left, view_list_tree(items), {"on_select": render_detail})
        tk.Button(left, text=f"New {noun}",
                  command=render_entry).pack(side=tk.BOTTOM, fill=tk.X)
        c.navigate(("list", noun))

    def switch(noun):
        state["noun"] = noun
        status_var.set(f"{app} · {noun}")
        # the tabs frame carries its tab index: the pane manager keys
        # each tab's own master/detail/popover histories off it
        c.navigate(("tabs", noun, nouns.index(noun)), clear_history=True)
        clear(right)
        render_list()

    for n in nouns:
        tk.Button(tabs, text=n, command=lambda n=n: switch(n)).pack(
            side=tk.LEFT, padx=2)
    c.navigate(("tabs", state["noun"], nouns.index(state["noun"])))
    render_list()
    if mainloop:
        root.mainloop()
    return root, c
