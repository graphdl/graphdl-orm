// arest-show — the WPF realization of the abstract UI (Samuel,
// 2026-07-08: "the arest cli should show Windows desktop apps using
// the same tricks iFactr does"; and "steal from iFactr — the Wpf
// target should be useful").
//
// Written fresh from iFactr-WPF's PATTERNS per the lift survey (the
// control classes there are ~90% boilerplate feeding a retired Canvas
// layout engine): Pane is a ContentControl over a manual view stack
// (Controls/Pane.cs), the shell is master | GridSplitter | detail
// with a tabs strip on top (WpfFactory.InitializeWindow), the entry
// form is a modal Window (PopoverPane), and the default metrics are
// iFactr's (PlatformDefaults: Segoe UI, cell 48, header #F8F8F8).
// iFactr-WPF is MIT (Zebra Technologies, 2017).
//
// The two tricks, same as the tk realization:
// - CONTROLS ARE DEFS: one constructor per abstract role in the
//   Controls registry ("control:list" ... ), the same layered
//   Register/Resolve discipline (MonoCross's MXContainer seam).
// - INVERSION VIA BIND: a control's Click never calls app code — it
//   >>= into the fact's own apply through the compiler host
//   (cli.py apply <app> <ft> <row>); the store is the state monad,
//   the panes re-render from D', refusals surface verbatim.
//
// usage: arest-show <apps-dir> <app> [noun]
using System.Diagnostics;
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace ArestShow;

// ---- the data seam: six verbs, two hosts (board task #36, 2026-07-11:
// swap the python subprocess for the native Rust resident) -- the SAME
// Register/Resolve discipline the control registry below uses, one
// level up: an interface for the six verbs, PyCli the original
// subprocess-per-call implementation (unchanged behavior), NativeServe
// a persistent `arest.exe --mcp` child. Program.Main resolves
// NativeServe by default; --py-host falls back to PyCli. ----
interface IHost
{
    JsonElement Schema();
    string[] Entities(string noun);
    (string id, string text, string value)[] Items(string noun);
    JsonElement Get(string noun, string id);
    JsonElement Actions(string noun, string id);
    JsonElement Apply(string ft, params string[] row);

    // Nouns is a PROJECTION of Schema (object_types filtered to entity
    // kinds), not a separate verb -- one shared reading, not forked
    // across both hosts.
    string[] Nouns() =>
        [.. Schema().GetProperty("object_types").EnumerateArray()
            .Where(n => n.GetProperty("kind").GetString() == "ObjectType")
            .Select(n => n.GetProperty("name").GetString()!)];
}

// the ONE landmark walk both hosts need: PyCli spawns cli.py directly;
// NativeServe finds rust/target/{release,debug}/arest.exe beside it.
static class EngineRoot
{
    public static readonly string Dir = Find();

    static string Find()
    {
        var d = AppContext.BaseDirectory;
        while (d != null && !File.Exists(Path.Combine(d, "cli.py")))
            d = Path.GetDirectoryName(d);
        return d ?? throw new FileNotFoundException(
            "cli.py not found above " + AppContext.BaseDirectory);
    }
}

// ---- PyCli: the original seam, unchanged behavior -- one
// `python cli.py <verb>` subprocess per call ----
sealed class PyCli(string appsDir, string app) : IHost
{
    JsonElement Call(params string[] args)
    {
        var psi = new ProcessStartInfo("python")
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        };
        foreach (var a in new[] { "-X", "utf8", Path.Combine(EngineRoot.Dir, "cli.py"),
                                  args[0], "--apps-dir", appsDir, app })
            psi.ArgumentList.Add(a);
        foreach (var a in args.Skip(1))
            psi.ArgumentList.Add(a);
        using var p = Process.Start(psi)!;
        var outp = p.StandardOutput.ReadToEnd();
        p.WaitForExit();
        var line = outp.Trim().Split('\n').LastOrDefault("") ?? "";
        return JsonDocument.Parse(line.Length > 0 ? line : "{}").RootElement;
    }

    public JsonElement Schema() => Call("schema");

    public string[] Entities(string noun) =>
        [.. Call("entities", noun).EnumerateArray()
            .Select(e => e.GetString()!)];

    public (string id, string text, string value)[] Items(string noun) =>
        [.. Call("items", noun).EnumerateArray()
            .Select(r => (r[0].GetString()!, r[1].GetString()!,
                          r[2].GetString()!))];

    public JsonElement Get(string noun, string id) => Call("get", noun, id);

    public JsonElement Actions(string noun, string id) =>
        Call("actions", noun, id);

    public JsonElement Apply(string ft, params string[] row) =>
        Call("apply", ft, JsonSerializer.Serialize(row));
}

// ---- NativeServe: the native Rust resident, ONE persistent
// `arest.exe --mcp --apps-dir <dir>` child speaking newline-delimited
// JSON-RPC 2.0 (main.rs's --mcp binding). TRANSPORT CHOICE: the --serve
// op table (base_seed/cells/compile_model/query/run_rules/sql_project/
// synthesize_pairs/verbs) has no schema/get/apply/actions op at all --
// only --mcp's tool table covers all six verbs, so --mcp is the one
// transport that reaches every verb with zero main.rs edits.
// apps_use preloads the app's store sidecar once (the {"d":...}
// preamble under the hood, main.rs's load_sidecar); schema/get/actions
// ride the native store_call arms directly and apply rides native_apply
// (falling back to the python delegate INSIDE arest.exe for an
// absorbed fact type -- transparent to this class, and to retract,
// which always delegates). Every write already reloads the resident's
// OWN in-memory store on the far side (apply_core / delegate_verb both
// refresh srv before answering), so this class never re-preambles
// after a write -- the next call simply reads the same child's already
// -current state.
// entities/items have NO mcp tool of their own (verified against
// main.rs's MCP_TOOLS/mcp_call_inner/store_call, 2026-07-11), so they
// compose shell-side from "query" (+ "actions" for the machine-tracked
// status column: a bare query on an ABSORBED status fact type would
// read the raw row_overwrite cell rather than the RMAP-resolved current
// value -- protocol.py's ft_view distinction, Registry.query's
// smStatusFt special case -- so status rides the same per-id "actions"
// read Registry.actions itself uses, paid only when the noun is
// actually machine-governed).
sealed class NativeServe : IHost, IDisposable
{
    // reads (query/get/actions/schema) answer in well under a second even
    // at the tasks app's ~1000-entity scale; apply's bounded derive is the
    // one call that can run long on a large, richly-derived corpus (a
    // 90s and a 600s attempt on the tasks app's own Task noun BOTH ran out
    // the clock still computing, board task #36's report has the detail),
    // so the timeout carries generous margin above the read path's actual
    // (sub-second) latency.
    static readonly TimeSpan Timeout = TimeSpan.FromSeconds(1800);

    readonly Process proc;
    readonly string app;
    int nextId;

    public NativeServe(string appsDir, string app)
    {
        this.app = app;
        var psi = new ProcessStartInfo(FindArestExe())
        {
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        };
        psi.ArgumentList.Add("--mcp");
        psi.ArgumentList.Add("--apps-dir");
        psi.ArgumentList.Add(appsDir);
        proc = Process.Start(psi)!;
        // drain stderr in the background so a full OS pipe buffer never
        // blocks the child on a diagnostic line this seam never reads
        var stderr = proc.StandardError;
        _ = Task.Run(() => { try { stderr.ReadToEnd(); } catch { } });

        // a throw anywhere below must still reap the child: a failed
        // constructor never returns an IHost/IDisposable for the caller
        // to clean up, so a leak here is a leak for the process's life
        try
        {
            RpcCall("initialize", new { protocolVersion = "2024-11-05" });
            var used = ToolCall("apps_use", new { name = app });
            if (!(used.TryGetProperty("ok", out var ok) && ok.ValueKind == JsonValueKind.True))
                throw new InvalidOperationException($"apps_use {app} failed: {used}");
        }
        catch
        {
            try { proc.Kill(entireProcessTree: true); } catch { }
            throw;
        }
    }

    static string FindArestExe()
    {
        var release = Path.Combine(EngineRoot.Dir, "rust", "target", "release", "arest.exe");
        if (File.Exists(release)) return release;
        var debug = Path.Combine(EngineRoot.Dir, "rust", "target", "debug", "arest.exe");
        if (File.Exists(debug)) return debug;
        throw new FileNotFoundException(
            $"arest.exe not found under {EngineRoot.Dir}/rust/target/{{release,debug}}; " +
            "run `cargo build` (or --release) in engine/rust, or pass --py-host");
    }

    JsonElement RpcCall(string method, object? paramsObj)
    {
        int id = ++nextId;
        object req = paramsObj is null
            ? new { jsonrpc = "2.0", id, method }
            : new { jsonrpc = "2.0", id, method, @params = paramsObj };
        var stdin = proc.StandardInput;
        stdin.Write(JsonSerializer.Serialize(req));
        stdin.Write('\n');
        stdin.Flush();
        while (true)
        {
            var readTask = proc.StandardOutput.ReadLineAsync();
            if (!readTask.Wait(Timeout))
                throw new TimeoutException(
                    $"arest --mcp did not answer {method} within {Timeout}");
            var line = readTask.Result
                ?? throw new EndOfStreamException("arest --mcp closed its output");
            if (line.Length == 0) continue;
            var root = JsonDocument.Parse(line).RootElement;
            if (!root.TryGetProperty("id", out var ridEl)
                || ridEl.ValueKind != JsonValueKind.Number
                || ridEl.GetInt32() != id)
                continue;                     // a stray line; keep reading for ours
            if (root.TryGetProperty("error", out var err))
                throw new InvalidOperationException($"arest --mcp {method}: {err}");
            return root.GetProperty("result").Clone();
        }
    }

    JsonElement ToolCall(string tool, object args)
    {
        var result = RpcCall("tools/call", new { name = tool, arguments = args });
        var text = result.GetProperty("content")[0].GetProperty("text").GetString()!;
        return JsonDocument.Parse(text).RootElement.Clone();
    }

    JsonElement[] Query(string factType) =>
        [.. ToolCall("query", new { fact_type = factType })
            .GetProperty("rows").EnumerateArray()];

    static string Scalar(JsonElement e) => e.ValueKind switch
    {
        JsonValueKind.String => e.GetString()!,
        JsonValueKind.Number => e.GetRawText(),
        _ => e.ToString(),
    };

    public JsonElement Schema() => ToolCall("schema", new { });

    public JsonElement Get(string noun, string id) => ToolCall("get", new { noun, id });

    public JsonElement Actions(string noun, string id) => ToolCall("actions", new { noun, id });

    public JsonElement Apply(string ft, params string[] row) =>
        ToolCall("apply", new { app, fact_type = ft, fact = row });

    // the test harness' cleanup path only (--task-roundtrip below) --
    // NOT one of the shell's six verbs, so it rides here rather than on
    // IHost; retract always delegates on the far side regardless.
    public JsonElement Retract(string ft, params string[] row) =>
        ToolCall("retract", new { app, fact_type = ft, fact = row });

    (string ft, int pos, string player)[] RoleRows() =>
        [.. Query("role")
            .Where(r => r.GetArrayLength() >= 4)
            .Select(r => (Scalar(r[1]), r[2].GetInt32(), Scalar(r[3])))];

    public string[] Entities(string noun)
    {
        // Registry.entities' exact union (protocol.py): the noun's own
        // population's keys, unioned with the role-1 population of
        // every fact type it heads -- an entity is an entity by
        // playing a fact.
        var keys = new SortedSet<string>(StringComparer.Ordinal);
        foreach (var row in Query(noun))
            if (row.GetArrayLength() > 0) keys.Add(Scalar(row[0]));
        foreach (var r in RoleRows())
            if (r.pos == 1 && r.player == noun)
                foreach (var row in Query(r.ft))
                    if (row.GetArrayLength() > 0) keys.Add(Scalar(row[0]));
        keys.Remove("");
        keys.Remove("φ");
        return [.. keys];
    }

    public (string id, string text, string value)[] Items(string noun)
    {
        var ids = Entities(noun);
        var roles = RoleRows();
        // the text column: the ALPHABETICALLY-FIRST (by fact type id)
        // binary fact type headed by noun. Role rows key as
        // "<ft>.<position>" (compiler.py's _role_facts), so
        // Registry.items' sort-over-role-tuples tie-break reduces
        // exactly to a plain ordinal string sort of the fact type id
        // among role-1 rows (the shared ".1" suffix never changes two
        // strings' relative order) -- proved 2026-07-11 rather than
        // assumed, since a wrong tie-break would silently pick the
        // wrong column.
        string? textFt = roles
            .Where(r => r.pos == 1 && r.player == noun)
            .Select(r => r.ft)
            .OrderBy(ft => ft, StringComparer.Ordinal)
            .FirstOrDefault(ft => roles.Any(q => q.ft == ft && q.pos == 2));
        var texts = new Dictionary<string, string>();
        if (textFt != null)
            foreach (var row in Query(textFt))
                if (row.GetArrayLength() >= 2)
                    texts.TryAdd(Scalar(row[0]), Scalar(row[1]));
        // the status column: machine_for's own walk (smDef + subtype),
        // paying the per-id "actions" call -- the only correct native
        // read of a row_overwrite-managed status column -- only when
        // the noun is actually governed by some machine.
        bool governed = Governed(noun);
        var outp = new (string, string, string)[ids.Length];
        for (int i = 0; i < ids.Length; i++)
        {
            var id = ids[i];
            string value = "";
            if (governed)
            {
                var acts = Actions(noun, id);
                if (acts.TryGetProperty("status", out var st)
                    && st.ValueKind == JsonValueKind.String)
                    value = st.GetString()!;
            }
            outp[i] = (id, texts.TryGetValue(id, out var t) ? t : id, value);
        }
        return outp;
    }

    bool Governed(string noun)
    {
        var bound = new HashSet<string>();
        foreach (var r in Query("smDef"))
            if (r.GetArrayLength() >= 2) bound.Add(Scalar(r[1]));
        var subs = new Dictionary<string, string>();
        foreach (var r in Query("subtype"))
            if (r.GetArrayLength() >= 2) subs[Scalar(r[0])] = Scalar(r[1]);
        var n = noun;
        var seen = new HashSet<string>();
        while (!bound.Contains(n) && subs.TryGetValue(n, out var sup) && seen.Add(n))
            n = sup;
        return bound.Contains(n);
    }

    public void Dispose()
    {
        try { proc.StandardInput.Close(); } catch { }
        try { if (!proc.WaitForExit(2000)) proc.Kill(entireProcessTree: true); } catch { }
        try { proc.Dispose(); } catch { }
    }
}

// ---- the pane: a ContentControl over a manual view stack ----
sealed class Pane : ContentControl
{
    readonly List<UIElement> stack = [];

    public void Push(UIElement view)
    {
        stack.Add(view);
        Content = view;
    }

    public void Pop()
    {
        if (stack.Count > 0) stack.RemoveAt(stack.Count - 1);
        Content = stack.Count > 0 ? stack[^1] : null;
    }

    public void Clear()
    {
        stack.Clear();
        Content = null;
    }
}

static class Defaults   // iFactr-WPF PlatformDefaults, copied as literals
{
    public const double CellHeight = 48;
    public static readonly FontFamily Font = new("Segoe UI");
    public static readonly Brush HeaderBg =
        new SolidColorBrush(Color.FromRgb(0xF8, 0xF8, 0xF8));
    public static readonly Brush SubText =
        new SolidColorBrush(Color.FromRgb(0x86, 0x86, 0x86));
}

sealed class Shell
{
    readonly IHost cli;
    readonly string app;
    string noun;
    readonly bool single;
    readonly Window window = new();
    readonly Pane master = new();
    readonly Pane detail = new();
    ColumnDefinition masterCol = null!;
    ColumnDefinition splitCol = null!;
    readonly TextBlock status = new()
    { Padding = new Thickness(6, 3, 6, 3), Background = Defaults.HeaderBg };

    // CONTROLS ARE DEFS: the per-role constructor registry — the same
    // Register/Resolve seam as kernel.register_form("control:<role>")
    readonly Dictionary<string, Func<JsonElement, UIElement>> controls;

    public Shell(IHost cli, string app, string noun, bool single = false)
    {
        this.cli = cli;
        this.app = app;
        this.noun = noun;
        this.single = single;
        controls = new()
        {
            ["control:list"] = RenderListControl,
            ["control:detail"] = RenderDetailControl,
            ["control:menu"] = RenderMenuControl,
        };
    }

    public Window Build(string[] nouns)
    {
        window.Title = $"arest — {app}";
        window.Width = 980;
        window.Height = 640;
        window.FontFamily = Defaults.Font;

        var tabs = new StackPanel
        { Orientation = Orientation.Horizontal, Background = Defaults.HeaderBg };
        foreach (var n in nouns)
        {
            var b = new Button
            { Content = n, Margin = new Thickness(3), Padding = new Thickness(10, 4, 10, 4) };
            b.Click += (_, _) => { noun = n; RenderList(); detail.Clear(); };
            tabs.Children.Add(b);
        }

        var grid = new Grid();
        masterCol = new ColumnDefinition
        { Width = new GridLength(1, GridUnitType.Star) };
        splitCol = new ColumnDefinition { Width = GridLength.Auto };
        grid.ColumnDefinitions.Add(masterCol);
        grid.ColumnDefinitions.Add(splitCol);
        grid.ColumnDefinitions.Add(new ColumnDefinition
        { Width = new GridLength(2.5, GridUnitType.Star) });
        Grid.SetColumn(master, 0);
        var split = new GridSplitter
        { Width = 4, HorizontalAlignment = HorizontalAlignment.Stretch };
        Grid.SetColumn(split, 1);
        Grid.SetColumn(detail, 2);
        grid.Children.Add(master);
        grid.Children.Add(split);
        grid.Children.Add(detail);

        var dock = new DockPanel();
        DockPanel.SetDock(tabs, Dock.Top);
        DockPanel.SetDock(status, Dock.Bottom);
        dock.Children.Add(tabs);
        dock.Children.Add(status);
        dock.Children.Add(grid);
        window.Content = dock;
        RenderList();
        // ADAPTIVE RENDERING: auto by width (FormFactor.SplitView's
        // rule), --single forces one pane; the detail covers the
        // master when narrow, Back restores it
        window.SizeChanged += (_, _) => ApplyMode();
        ApplyMode();
        return window;
    }

    bool IsSplit() => !single && window.ActualWidth >= 680;

    void ApplyMode()
    {
        if (masterCol == null) return;
        bool split = IsSplit();
        bool detailShowing = detail.Content != null;
        if (split || !detailShowing)
        {
            master.Visibility = Visibility.Visible;
            masterCol.Width = new GridLength(1, GridUnitType.Star);
            splitCol.Width = GridLength.Auto;
        }
        else
        {
            master.Visibility = Visibility.Collapsed;
            masterCol.Width = new GridLength(0);
            splitCol.Width = new GridLength(0);
        }
    }

    void Status(string s) => status.Text = s;

    // -- the master pane: the list perspective + New --
    public void RenderList()
    {
        var doc = JsonSerializer.SerializeToElement(
            cli.Items(noun).Select(r => new[] { r.id, r.text, r.value }));
        var panel = new DockPanel();
        var neu = new Button
        { Content = $"New {noun}", Margin = new Thickness(4) };
        neu.Click += (_, _) => RenderEntry();
        DockPanel.SetDock(neu, Dock.Bottom);
        panel.Children.Add(neu);
        panel.Children.Add(controls["control:list"](doc));
        master.Push(panel);
        Status($"{app} · {noun}");
    }

    // the shared ContentCell recipe (uicells): ⟨auto | star text |
    // auto value⟩ over two auto rows, subtext spanning — each cell an
    // engine-laid Canvas, exactly the tk realization
    UIElement RenderListControl(JsonElement items)
    {
        var stack = new StackPanel();
        const double width = 300;
        foreach (var it in items.EnumerateArray())
        {
            string id = it[0].GetString()!, text = it[1].GetString()!,
                value = it.GetArrayLength() > 2 ? it[2].GetString()! : "";
            var cellCanvas = new Canvas { Width = width };
            var g = new LayoutGrid
            {
                Rows = [new Track(Unit.Auto), new Track(Unit.Auto)],
                Columns = [new Track(Unit.Auto), new Track(Unit.Star),
                           new Track(Unit.Auto)],
                Padding = (16, 10, 16, 10),
            };
            var t = new TextBlock
            { Text = text, TextWrapping = TextWrapping.NoWrap };
            var s = new TextBlock
            {
                Text = id, FontSize = 10.5,
                Foreground = Defaults.SubText,
            };
            g.Add(El(t, cellCanvas, 0, 1, Align.Near, Align.Center,
                     (0, 0, 0, 0), isLabel: true));
            g.Add(El(s, cellCanvas, 1, 1, Align.Near, Align.Center,
                     (0, 0, 0, 0), isLabel: true));
            if (value.Length > 0)
            {
                var v = new TextBlock
                { Text = value, Foreground = Defaults.SubText };
                g.Add(El(v, cellCanvas, 0, 2, Align.Far, Align.Center,
                         (7, 0, 0, 0), isLabel: true));
            }
            var (_w, h) = Layout.Perform(
                g, (width, Defaults.CellHeight),
                (width, double.PositiveInfinity));
            cellCanvas.Height = h;
            var cid = id;
            cellCanvas.MouseLeftButtonUp += (_, _) => RenderDetail(cid);
            stack.Children.Add(cellCanvas);
            stack.Children.Add(new Border
            {
                Height = 1,
                Background = new SolidColorBrush(
                    Color.FromRgb(0xE4, 0xE4, 0xE4)),
            });
        }
        return new ScrollViewer { Content = stack };
    }

    // -- the detail pane: fields + the machine menu --
    void RenderDetail(string id)
    {
        var got = cli.Get(noun, id);
        var panel = new DockPanel();
        var bar = new StackPanel
        { Orientation = Orientation.Horizontal };
        var back = new Button
        { Content = "\u25C0 Back", Margin = new Thickness(3), Padding = new Thickness(8, 2, 8, 2) };
        back.Click += (_, _) =>
        {
            detail.Pop();
            ApplyMode();                    // empty detail restores master
        };
        bar.Children.Add(back);
        DockPanel.SetDock(bar, Dock.Top);
        panel.Children.Add(bar);
        var acts = cli.Actions(noun, id);
        if (acts.TryGetProperty("status", out var st)
            && st.ValueKind == JsonValueKind.String)
        {
            var menu = controls["control:menu"](acts);
            DockPanel.SetDock(menu, Dock.Bottom);
            panel.Children.Add(menu);
            // the menu's buttons close over (noun, id) via Tag
            foreach (var b in ((StackPanel)menu).Children.OfType<Button>())
            {
                var ft = (string)b.Tag;
                b.Click += (_, _) =>
                {
                    // THE BIND: the click IS the fact's function
                    var r = cli.Apply(ft, id);
                    Status(r.TryGetProperty("committed", out var ok)
                           && ok.GetBoolean()
                        ? $"committed {ft}"
                        : "REFUSED: " + r.GetProperty("violations").ToString());
                    RenderList();
                    RenderDetail(id);
                };
            }
        }
        panel.Children.Add(controls["control:detail"](got));
        detail.Push(panel);
        ApplyMode();                        // single pane: detail covers
    }

    // -- THE ABSTRACT ENGINE over a Canvas (the WPF read's contract:
    //    Measure -> FrameworkElement.DesiredSize, place ->
    //    Canvas.SetLeft/SetTop; text rounds up like iFactr's Label) --
    static LayoutElement El(FrameworkElement fe, Canvas host,
                            int row, int col, Align halign, Align valign,
                            (double, double, double, double) margin,
                            bool isLabel = false)
    {
        host.Children.Add(fe);
        return new LayoutElement
        {
            Measure = (cw, ch) =>
            {
                fe.Width = double.NaN;
                fe.Height = double.NaN;
                fe.Measure(new Size(cw >= 1e307
                        ? double.PositiveInfinity : cw,
                    ch >= 1e307 ? double.PositiveInfinity : ch));
                return (fe.DesiredSize.Width, fe.DesiredSize.Height);
            },
            Place = (x, y, w, h) =>
            {
                bool text = fe is TextBlock;
                fe.Width = text ? Math.Ceiling(w) : w;
                fe.Height = text ? Math.Ceiling(h) : h;
                Canvas.SetLeft(fe, x);
                Canvas.SetTop(fe, y);
            },
            Row = row,
            Col = col,
            HAlign = halign,
            VAlign = valign,
            Margin = margin,
            IsLabel = isLabel,
        };
    }

    UIElement RunLayout(LayoutGrid grid, Canvas canvas)
    {
        double width = Math.Max(detail.ActualWidth, 420);
        var (w, h) = Layout.Perform(grid, (width, 0),
                                    (width, double.PositiveInfinity));
        canvas.Width = w;
        canvas.Height = h;
        return new ScrollViewer { Content = canvas };
    }

    UIElement RenderDetailControl(JsonElement got)
    {
        var canvas = new Canvas();
        var grid = new LayoutGrid
        {
            Columns = [new Track(Unit.Auto), new Track(Unit.Star)],
            Padding = (8, 4, 8, 4),
        };
        int row = 0;
        if (got.TryGetProperty("fields", out var fields))
            foreach (var f in fields.EnumerateObject()
                     .Where(f => f.Value.ValueKind is not
                            (JsonValueKind.True or JsonValueKind.False))
                     .OrderBy(f => f.Name))
            {
                grid.Rows.Add(new Track(Unit.Auto));
                var k = new TextBlock
                { Text = f.Name + ":", FontWeight = FontWeights.Bold };
                var v = new TextBlock
                {
                    Text = f.Value.ValueKind == JsonValueKind.Null
                        ? "" : f.Value.ToString(),
                    TextWrapping = TextWrapping.Wrap,
                };
                grid.Add(El(k, canvas, row, 0, Align.Near, Align.Near,
                            (0, 2, 8, 2)));
                grid.Add(El(v, canvas, row, 1, Align.Near, Align.Near,
                            (0, 2, 0, 2), isLabel: true));
                row++;
            }
        return RunLayout(grid, canvas);
    }

    UIElement RenderMenuControl(JsonElement acts)
    {
        var bar = new StackPanel
        { Orientation = Orientation.Horizontal, Margin = new Thickness(4) };
        if (acts.TryGetProperty("actions", out var list))
            foreach (var a in list.EnumerateArray())
            {
                var ev = a.GetProperty("event").GetString()!;
                var to = a.GetProperty("to").GetString();
                bar.Children.Add(new Button
                {
                    Content = $"{ev} → {to}",
                    Tag = ev,
                    Margin = new Thickness(3),
                    Padding = new Thickness(8, 3, 8, 3),
                });
            }
        return bar;
    }

    // -- the entry popover: a modal Window (iFactr's PopoverPane) --
    void RenderEntry()
    {
        var top = new Window
        {
            Title = $"New {noun}",
            Owner = window,
            Width = 560,
            SizeToContent = SizeToContent.Height,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ResizeMode = ResizeMode.NoResize,
            FontFamily = Defaults.Font,
        };
        // the entry tree's inputs come from get's field surface on a
        // fresh probe id? No — the schema's classified columns arrive
        // through the same read the tk realization uses: the entry
        // tree is canon-derived host-side; HERE the cli's schema
        // answers fact types and the container filters the noun's
        // functional ones (role-1 = noun, binary or unary).
        var schema = cli.Schema();
        var canvas = new Canvas();
        var grid = new LayoutGrid
        {
            Columns = [new Track(Unit.Auto), new Track(Unit.Star)],
            Padding = (10, 6, 10, 6),
        };
        var inputs = new List<(string ft, bool unary, Func<string> read)>();
        int row = 0;

        void AddRow(string label, FrameworkElement input,
                    Align halign = Align.Stretch)
        {
            grid.Rows.Add(new Track(Unit.Auto));
            var k = new TextBlock { Text = label + ":" };
            grid.Add(El(k, canvas, row, 0, Align.Near, Align.Near,
                        (0, 3, 8, 3)));
            grid.Add(El(input, canvas, row, 1, halign, Align.Near,
                        (0, 3, 0, 3)));
            row++;
        }

        var idBox = new TextBox { MinWidth = 320 };
        AddRow("id", idBox);
        foreach (var ft in schema.GetProperty("fact_types").EnumerateArray())
        {
            var roles = ft.GetProperty("roles").EnumerateArray()
                .Select(r => r.GetString()).ToArray();
            if (roles.Length == 0 || roles[0] != noun || roles.Length > 2)
                continue;
            var id = ft.GetProperty("id").GetString()!;
            if (roles.Length == 1)
            {
                var cb = new CheckBox();
                inputs.Add((id, true, () => cb.IsChecked == true ? "T" : ""));
                AddRow(id, cb, Align.Near);
            }
            else
            {
                var tb = new TextBox { MinWidth = 320 };
                inputs.Add((id, false, () => tb.Text));
                AddRow(roles[1], tb);
            }
        }
        var create = new Button
        { Content = "Create", Padding = new Thickness(10, 4, 10, 4) };
        create.Click += (_, _) =>
        {
            var id = idBox.Text;
            if (string.IsNullOrWhiteSpace(id))
            { Status("REFUSED: an id is required"); return; }
            foreach (var (ft, unary, read) in inputs)
            {
                var v = read();
                if (string.IsNullOrEmpty(v)) continue;
                var r = unary ? cli.Apply(ft, id) : cli.Apply(ft, id, v);
                if (!(r.TryGetProperty("committed", out var ok)
                      && ok.GetBoolean()))
                {
                    Status("REFUSED: "
                           + r.GetProperty("violations").ToString());
                    return;
                }
            }
            Status($"created {id}");
            top.Close();
            RenderList();
            RenderDetail(id);
        };
        grid.Rows.Add(new Track(Unit.Auto));
        grid.Add(El(create, canvas, row, 1, Align.Near, Align.Near,
                    (0, 10, 0, 0)));
        var (w, h) = Layout.Perform(grid, (520, 0),
                                    (520, double.PositiveInfinity));
        canvas.Width = w;
        canvas.Height = h;
        top.Content = new ScrollViewer { Content = canvas };
        top.ShowDialog();
    }
}

static class Program
{
    [STAThread]
    static int Main(string[] args)
    {
        if (args.Contains("--layout-selftest"))
        {
            // the cross-host twin gate: the same seven golden cases as
            // tests/test_uilayout.py
            return LayoutSelfTest.Run();
        }
        // --host-diff and --task-roundtrip are the board task #36
        // acceptance harnesses: no WPF window, just the IHost seam
        // itself, so they run headless (see the task's report for why:
        // this sandbox has no interactive window station to pump).
        if (args.Length >= 1 && args[0] == "--host-diff")
            return args.Length >= 3
                ? HostDiff.Run(args[1], args[2], args.Length > 3 ? args[3..] : null)
                : Usage();
        if (args.Length >= 1 && args[0] == "--task-roundtrip")
            return args.Length >= 7
                ? TaskRoundtrip.Run(args[1], args[2], args[3], args[4], args[5], args[6])
                : Usage();
        if (args.Length < 2)
            return Usage();
        // NativeServe by default (the seam this task swaps in); --py-host
        // falls back to the original subprocess-per-call PyCli.
        IHost cli = args.Contains("--py-host")
            ? new PyCli(args[0], args[1])
            : new NativeServe(args[0], args[1]);
        try
        {
            var nouns = cli.Nouns();
            if (nouns.Length == 0)
            {
                Console.Error.WriteLine($"app {args[1]} has no entity nouns");
                return 1;
            }
            var noun = args.Length > 2 && !args[2].StartsWith("--")
                ? args[2] : nouns[0];
            var app = new Application();
            var shell = new Shell(cli, args[1], noun,
                                  single: args.Contains("--single"));
            var window = shell.Build(nouns);
            if (args.Contains("--probe"))
            {
                // construct + measure once, no pump: the build smoke
                window.Show();
                window.Close();
                Console.WriteLine("probe ok: " + string.Join(",", nouns));
                return 0;
            }
            return app.Run(window);
        }
        finally
        {
            (cli as IDisposable)?.Dispose();
        }
    }

    static int Usage()
    {
        Console.Error.WriteLine(
            "usage: arest-show <apps-dir> <app> [noun] [--probe | --single | --py-host]\n" +
            "       arest-show --layout-selftest\n" +
            "       arest-show --host-diff <apps-dir> <app> [noun...]\n" +
            "       arest-show --task-roundtrip <apps-dir> <app> <noun> <id> <forward-ft> <backward-ft>");
        return 2;
    }
}

// ---- --host-diff: the transport differential (board task #36,
// acceptance a) -- drives PyCli and NativeServe through the SAME verb
// sequence (nouns, entities, items, get, actions) and asserts JSON-equal
// answers: object keys compared order-blind, arrays compared
// order-sensitive (this domain's lists -- entities, items, actions --
// are all meaningfully ordered). ----
static class HostDiff
{
    // nouns: when given, scopes the entities/items/get/actions sweep to
    // just these (the "nouns" comparison itself always covers the FULL
    // list either way) -- an app built on the shared base canon reflects
    // upward of a hundred metamodel nouns alongside its own domain ones,
    // and PyCli's subprocess-per-call cost makes an exhaustive sweep
    // impractical for what the board task calls "a small script".
    public static int Run(string appsDir, string app, string[]? nouns = null)
    {
        IHost py = new PyCli(appsDir, app);
        IHost native = new NativeServe(appsDir, app);
        try
        {
            bool ok = true;
            var pyNouns = py.Nouns();
            ok &= Check("nouns",
                JsonSerializer.SerializeToElement(pyNouns),
                JsonSerializer.SerializeToElement(native.Nouns()));
            foreach (var noun in nouns ?? pyNouns)
            {
                ok &= Check($"entities({noun})",
                    JsonSerializer.SerializeToElement(py.Entities(noun)),
                    JsonSerializer.SerializeToElement(native.Entities(noun)));
                var pyItems = py.Items(noun).Select(t => new[] { t.id, t.text, t.value }).ToArray();
                var nativeItems = native.Items(noun).Select(t => new[] { t.id, t.text, t.value }).ToArray();
                ok &= Check($"items({noun})",
                    JsonSerializer.SerializeToElement(pyItems),
                    JsonSerializer.SerializeToElement(nativeItems));
                var ids = py.Entities(noun);
                if (ids.Length > 0)
                {
                    var id = ids[0];
                    ok &= Check($"get({noun},{id})", py.Get(noun, id), native.Get(noun, id));
                    ok &= Check($"actions({noun},{id})", py.Actions(noun, id), native.Actions(noun, id));
                }
            }
            Console.WriteLine(ok ? "HOST-DIFF: PASS" : "HOST-DIFF: FAIL");
            return ok ? 0 : 1;
        }
        finally
        {
            (py as IDisposable)?.Dispose();
            (native as IDisposable)?.Dispose();
        }
    }

    static bool Check(string label, JsonElement a, JsonElement b)
    {
        bool eq = JsonEquals(a, b);
        Console.WriteLine((eq ? "  ok   " : "  FAIL ") + label);
        if (!eq)
        {
            Console.WriteLine("    py:     " + a.GetRawText());
            Console.WriteLine("    native: " + b.GetRawText());
        }
        return eq;
    }

    public static bool JsonEquals(JsonElement a, JsonElement b)
    {
        if (a.ValueKind != b.ValueKind)
            return a.ToString() == b.ToString();
        switch (a.ValueKind)
        {
            case JsonValueKind.Object:
                var ap = a.EnumerateObject().ToDictionary(p => p.Name, p => p.Value);
                var bp = b.EnumerateObject().ToDictionary(p => p.Name, p => p.Value);
                if (ap.Count != bp.Count) return false;
                foreach (var (k, av) in ap)
                    if (!bp.TryGetValue(k, out var bv) || !JsonEquals(av, bv)) return false;
                return true;
            case JsonValueKind.Array:
                var al = a.EnumerateArray().ToArray();
                var bl = b.EnumerateArray().ToArray();
                if (al.Length != bl.Length) return false;
                for (int i = 0; i < al.Length; i++)
                    if (!JsonEquals(al[i], bl[i])) return false;
                return true;
            default:
                return a.ToString() == b.ToString();
        }
    }
}

// ---- --task-roundtrip: the tasks-app apply/verify/retract evidence
// (board task #36, acceptance b) -- drives one forward transition and
// its retraction through NativeServe ONLY (the native path), printing
// each receipt so the caller can confirm the pane would re-render the
// new status and the event log grew by exactly one line. Byte-restoring
// the app's files on disk is the CALLER's job (snapshot/hash outside
// this process, since a retract recompiles rather than reverting the
// log in place); this tool proves the seam's read-your-write behavior
// over the real store. ----
static class TaskRoundtrip
{
    public static int Run(string appsDir, string app, string noun, string id,
                          string forwardFt, string backwardFt)
    {
        var eventsPath = Path.Combine(appsDir, app, $"{app}.events.jsonl");
        long LineCount() => File.Exists(eventsPath) ? File.ReadLines(eventsPath).LongCount() : 0;

        var native = new NativeServe(appsDir, app);
        try
        {
            long before = LineCount();
            var actionsBefore = native.Actions(noun, id);
            Console.WriteLine("before:   " + actionsBefore.GetRawText());

            var applied = native.Apply(forwardFt, id);
            Console.WriteLine("apply:    " + applied.GetRawText());
            bool committed = applied.TryGetProperty("committed", out var c1)
                && c1.ValueKind == JsonValueKind.True;
            long afterApply = LineCount();

            var actionsAfter = native.Actions(noun, id);
            Console.WriteLine("after:    " + actionsAfter.GetRawText());

            var retracted = native.Retract(forwardFt, id);
            Console.WriteLine("retract:  " + retracted.GetRawText());
            bool retractCommitted = retracted.TryGetProperty("committed", out var c2)
                && c2.ValueKind == JsonValueKind.True;

            var actionsRestored = native.Actions(noun, id);
            Console.WriteLine("restored: " + actionsRestored.GetRawText());

            string? statusBefore = actionsBefore.GetProperty("status").GetString();
            string? statusAfter = actionsAfter.GetProperty("status").GetString();
            string? statusRestored = actionsRestored.GetProperty("status").GetString();

            bool logGrewByOne = (afterApply - before) == 1;
            bool statusChanged = statusBefore != statusAfter;
            bool statusReverted = statusBefore == statusRestored;

            Console.WriteLine(
                $"committed={committed} logGrewByOne={logGrewByOne} " +
                $"({before}->{afterApply}) statusChanged={statusBefore}->{statusAfter} " +
                $"retractCommitted={retractCommitted} statusReverted={statusReverted}");
            bool ok = committed && logGrewByOne && statusChanged
                     && retractCommitted && statusReverted;
            Console.WriteLine(ok ? "TASK-ROUNDTRIP: PASS" : "TASK-ROUNDTRIP: FAIL");
            return ok ? 0 : 1;
        }
        finally
        {
            native.Dispose();
        }
    }
}
