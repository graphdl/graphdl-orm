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

// ---- the compiler-host delegation: every verb is one cli.py call ----
sealed class Cli(string appsDir, string app)
{
    static readonly string Root = FindRoot();

    static string FindRoot()
    {
        var d = AppContext.BaseDirectory;
        while (d != null && !File.Exists(Path.Combine(d, "cli.py")))
            d = Path.GetDirectoryName(d);
        return d ?? throw new FileNotFoundException(
            "cli.py not found above " + AppContext.BaseDirectory);
    }

    public JsonElement Call(params string[] args)
    {
        var psi = new ProcessStartInfo("python")
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        };
        foreach (var a in new[] { "-X", "utf8", Path.Combine(Root, "cli.py"),
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

    public string[] Nouns() =>
        [.. Call("schema").GetProperty("object_types").EnumerateArray()
            .Where(n => n.GetProperty("kind").GetString() == "ObjectType")
            .Select(n => n.GetProperty("name").GetString()!)];

    public string[] Entities(string noun) =>
        [.. Call("entities", noun).EnumerateArray()
            .Select(e => e.GetString()!)];

    public JsonElement Get(string noun, string id) => Call("get", noun, id);

    public JsonElement Actions(string noun, string id) =>
        Call("actions", noun, id);

    public JsonElement Apply(string ft, params string[] row) =>
        Call("apply", ft, JsonSerializer.Serialize(row));
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
    readonly Cli cli;
    readonly string app;
    string noun;
    readonly Window window = new();
    readonly Pane master = new();
    readonly Pane detail = new();
    readonly TextBlock status = new()
    { Padding = new Thickness(6, 3, 6, 3), Background = Defaults.HeaderBg };

    // CONTROLS ARE DEFS: the per-role constructor registry — the same
    // Register/Resolve seam as kernel.register_form("control:<role>")
    readonly Dictionary<string, Func<JsonElement, UIElement>> controls;

    public Shell(Cli cli, string app, string noun)
    {
        this.cli = cli;
        this.app = app;
        this.noun = noun;
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
        grid.ColumnDefinitions.Add(new ColumnDefinition
        { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition
        { Width = GridLength.Auto });
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
        return window;
    }

    void Status(string s) => status.Text = s;

    // -- the master pane: the list perspective + New --
    public void RenderList()
    {
        var doc = JsonSerializer.SerializeToElement(
            cli.Entities(noun).Select(i => new[] { i, i }));
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

    UIElement RenderListControl(JsonElement items)
    {
        var box = new ListBox { BorderThickness = new Thickness(0) };
        foreach (var it in items.EnumerateArray())
        {
            var id = it[0].GetString();
            box.Items.Add(new ListBoxItem
            { Content = id, Tag = id, Height = double.NaN });
        }
        box.SelectionChanged += (_, _) =>
        {
            if (box.SelectedItem is ListBoxItem { Tag: string id })
                RenderDetail(id);
        };
        return box;
    }

    // -- the detail pane: fields + the machine menu --
    void RenderDetail(string id)
    {
        var got = cli.Get(noun, id);
        var panel = new DockPanel();
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
        var schema = cli.Call("schema");
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
        if (args.Length < 2)
        {
            Console.Error.WriteLine("usage: arest-show <apps-dir> <app> [noun] [--probe | --layout-selftest]");
            return 2;
        }
        var cli = new Cli(args[0], args[1]);
        var nouns = cli.Nouns();
        if (nouns.Length == 0)
        {
            Console.Error.WriteLine($"app {args[1]} has no entity nouns");
            return 1;
        }
        var noun = args.Length > 2 && !args[2].StartsWith("--")
            ? args[2] : nouns[0];
        var app = new Application();
        var shell = new Shell(cli, args[1], noun);
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
}
