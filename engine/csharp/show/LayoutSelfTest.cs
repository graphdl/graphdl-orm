// The layout twin gate: the SAME seven golden cases as
// tests/test_uilayout.py, run against the C# engine. A python pin
// invokes `arest-show --layout-selftest` and asserts exit 0 — the two
// engines stay certified-equal, one meaning.
namespace ArestShow;

static class LayoutSelfTest
{
    sealed class Box
    {
        public double W, H;
        public (double x, double y, double w, double h) Rect;
        public LayoutElement El = null!;

        public static Box Make(double w, double h,
                               Action<LayoutElement>? cfg = null,
                               bool wrap = false)
        {
            var b = new Box { W = w, H = h };
            b.El = new LayoutElement
            {
                Measure = (cw, ch) =>
                {
                    if (!wrap)
                        return (Math.Min(b.W, cw), Math.Min(b.H, ch));
                    double ww = Math.Min(b.W, cw);
                    double hh = Math.Min(b.H * (b.W / Math.Max(ww, 1.0)), ch);
                    return (ww, hh);
                },
                Place = (x, y, w2, h2) => b.Rect = (x, y, w2, h2),
            };
            cfg?.Invoke(b.El);
            return b;
        }
    }

    static int failures;

    static void Check(string name, bool ok)
    {
        if (!ok)
        {
            failures++;
            Console.Error.WriteLine("FAIL " + name);
        }
    }

    static bool Eq((double x, double y, double w, double h) r,
                   double x, double y, double w, double h) =>
        Math.Abs(r.x - x) < 0.01 && Math.Abs(r.y - y) < 0.01
        && Math.Abs(r.w - w) < 0.01 && Math.Abs(r.h - h) < 0.01;

    public static int Run()
    {
        double inf = double.PositiveInfinity;

        // 1. label column + star value column
        {
            var g = new LayoutGrid
            {
                Columns = [new Track(Unit.Auto), new Track(Unit.Star)],
                Rows = [new Track(Unit.Auto), new Track(Unit.Auto)],
            };
            var k1 = Box.Make(40, 20, e => { e.Row = 0; e.Col = 0; e.HAlign = Align.Near; e.VAlign = Align.Near; });
            var v1 = Box.Make(500, 20, e => { e.Row = 0; e.Col = 1; });
            var k2 = Box.Make(60, 20, e => { e.Row = 1; e.Col = 0; e.HAlign = Align.Near; e.VAlign = Align.Near; });
            var v2 = Box.Make(30, 20, e => { e.Row = 1; e.Col = 1; e.HAlign = Align.Near; e.VAlign = Align.Near; });
            foreach (var b in new[] { k1, v1, k2, v2 }) g.Add(b.El);
            var (w, h) = Layout.Perform(g, (0, 0), (300, inf));
            Check("detail.size", w == 300 && h == 40);
            Check("detail.k1", Eq(k1.Rect, 0, 0, 40, 20));
            Check("detail.k2", Eq(k2.Rect, 0, 20, 60, 20));
            Check("detail.v1", Eq(v1.Rect, 60, 0, 240, 20));
            Check("detail.v2", v2.Rect.x == 60 && v2.Rect.w == 30);
        }

        // 2. absolute + weighted stars + padding
        {
            var g = new LayoutGrid
            {
                Columns = [new Track(Unit.Absolute, 50), new Track(Unit.Star, 1), new Track(Unit.Star, 3)],
                Rows = [new Track(Unit.Absolute, 30)],
                Padding = (10, 5, 10, 5),
            };
            var a = Box.Make(999, 999, e => { e.Row = 0; e.Col = 0; });
            var b = Box.Make(999, 999, e => { e.Row = 0; e.Col = 1; });
            var c = Box.Make(999, 999, e => { e.Row = 0; e.Col = 2; });
            foreach (var x in new[] { a, b, c }) g.Add(x.El);
            Layout.Perform(g, (0, 0), (270, 40));
            Check("stars.a", Eq(a.Rect, 10, 5, 50, 30));
            Check("stars.b", Eq(b.Rect, 60, 5, 50, 30));
            Check("stars.c", Eq(c.Rect, 110, 5, 150, 30));
        }

        // 3. margins + alignments
        {
            var g = new LayoutGrid
            {
                Columns = [new Track(Unit.Absolute, 100)],
                Rows = [new Track(Unit.Absolute, 100)],
            };
            var near = Box.Make(20, 10, e => { e.Row = 0; e.Col = 0; e.HAlign = Align.Near; e.VAlign = Align.Near; e.Margin = (5, 6, 0, 0); });
            var far = Box.Make(20, 10, e => { e.Row = 0; e.Col = 0; e.HAlign = Align.Far; e.VAlign = Align.Far; e.Margin = (0, 0, 5, 6); });
            var ctr = Box.Make(20, 10, e => { e.Row = 0; e.Col = 0; e.HAlign = Align.Center; e.VAlign = Align.Center; });
            var fill = Box.Make(20, 10, e => { e.Row = 0; e.Col = 0; });
            foreach (var x in new[] { near, far, ctr, fill }) g.Add(x.El);
            Layout.Perform(g, (0, 0), (100, 100));
            Check("align.near", Eq(near.Rect, 5, 6, 20, 10));
            Check("align.far", Eq(far.Rect, 75, 84, 20, 10));
            Check("align.center", Eq(ctr.Rect, 40, 45, 20, 10));
            Check("align.fill", Eq(fill.Rect, 0, 0, 100, 100));
        }

        // 4. auto placement flows rows
        {
            var g = new LayoutGrid
            { Columns = [new Track(Unit.Star), new Track(Unit.Star)] };
            var boxes = Enumerable.Range(0, 5)
                .Select(_ => Box.Make(10, 10)).ToArray();
            foreach (var b in boxes) g.Add(b.El);
            Layout.Perform(g, (0, 0), (100, inf));
            var cells = boxes.Select(b => (b.Rect.x, b.Rect.y)).ToArray();
            var want = new[] { (0.0, 0.0), (50.0, 0.0), (0.0, 10.0),
                               (50.0, 10.0), (0.0, 20.0) };
            Check("flow", cells.SequenceEqual(want));
        }

        // 5. the label reflow grows its auto row
        {
            var g = new LayoutGrid
            { Columns = [new Track(Unit.Star)], Rows = [new Track(Unit.Auto)] };
            var t = Box.Make(200, 20, e => { e.Row = 0; e.Col = 0; e.IsLabel = true; e.HAlign = Align.Near; e.VAlign = Align.Near; }, wrap: true);
            g.Add(t.El);
            var (_, h) = Layout.Perform(g, (0, 0), (100, inf));
            Check("reflow", t.Rect.h == 40.0 && h == 40.0);
        }

        // 6. scale multiplies heights, margins, padding
        {
            var g = new LayoutGrid
            {
                Columns = [new Track(Unit.Star)],
                Rows = [new Track(Unit.Absolute, 10)],
                Padding = (2, 3, 2, 3),
            };
            var b = Box.Make(50, 999, e => { e.Row = 0; e.Col = 0; e.Margin = (1, 1, 1, 1); });
            g.Add(b.El);
            Layout.Perform(g, (0, 0), (104, 100), scale: 2.0);
            Check("scale", Eq(b.Rect, 2 * 2 + 1 * 2, 3 * 2 + 1 * 2,
                              104 - 4 * 2 - 2 * 2, 10 * 2 - 2 * 2));
        }

        // 7. minimum stretches the last auto
        {
            var g = new LayoutGrid
            {
                Columns = [new Track(Unit.Auto), new Track(Unit.Auto)],
                Rows = [new Track(Unit.Auto)],
            };
            var a = Box.Make(30, 10, e => { e.Row = 0; e.Col = 0; e.HAlign = Align.Near; e.VAlign = Align.Near; });
            var b = Box.Make(30, 10, e => { e.Row = 0; e.Col = 1; e.HAlign = Align.Near; e.VAlign = Align.Near; });
            g.Add(a.El);
            g.Add(b.El);
            var (w, _) = Layout.Perform(g, (200, 0), (300, 50));
            Check("minstretch", w == 200 && b.Rect.x == 30);
        }

        if (failures == 0)
        {
            Console.WriteLine("layout-selftest ok: 7 cases");
            return 0;
        }
        Console.Error.WriteLine($"layout-selftest: {failures} failures");
        return 1;
    }
}
