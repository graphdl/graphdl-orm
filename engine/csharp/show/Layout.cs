// The abstract layout engine — the C# transliteration of
// pyarest.uilayout (itself the faithful port of iFactr's
// GridExtensions.PerformLayout; Samuel 2026-07-08: these are the
// abstract invariants). Layout computes ONCE here; WPF supplies
// exactly Measure -> FrameworkElement.DesiredSize and place ->
// Canvas.SetLeft/SetTop (the WPF read: no DPI math, 1/96" native).
// The --layout-selftest golden cases twin this engine against the
// python pins (test_uilayout.py) — certified-equal, one meaning.
namespace ArestShow;

enum Unit { Absolute, Auto, Star }

enum Align { Stretch, Near, Far, Center }

sealed class Track(Unit unit = Unit.Auto, double weight = 1.0)
{
    public Unit Unit = unit;
    public double Weight = weight;
}

sealed class LayoutElement
{
    public const int AutoIndex = -1;

    public required Func<double, double, (double, double)> Measure;
    public required Action<double, double, double, double> Place;
    public int Row = AutoIndex;
    public int Col = AutoIndex;
    public int RowSpan = 1;
    public int ColSpan = 1;
    public (double L, double T, double R, double B) Margin = (0, 0, 0, 0);
    public Align HAlign = Align.Stretch;
    public Align VAlign = Align.Stretch;
    public bool Visible = true;
    public bool IsLabel = false;
}

sealed class LayoutGrid
{
    public List<Track> Rows = [];
    public List<Track> Columns = [];
    public (double L, double T, double R, double B) Padding = (0, 0, 0, 0);
    public List<LayoutElement> Children = [];

    public LayoutElement Add(LayoutElement e)
    {
        Children.Add(e);
        return e;
    }
}

static class Layout
{
    const double Big = 1e308;

    sealed class Space
    {
        public double Origin;
        public double Size;
    }

    static List<(LayoutElement e, (int r, int c) at)> Blocked(
        Dictionary<LayoutElement, (int r, int c)> indices,
        int row, int col, LayoutElement element)
    {
        var outp = new List<(LayoutElement, (int, int))>();
        foreach (var (e, (r, c)) in indices)
        {
            if (ReferenceEquals(e, element)) continue;
            if (c <= col + element.ColSpan - 1 && c + e.ColSpan > col
                && r <= row + element.RowSpan - 1 && r + e.RowSpan > row)
                outp.Add((e, (r, c)));
        }
        return outp;
    }

    static int ColumnForRow(int row, int ncols, LayoutElement element,
                            Dictionary<LayoutElement, (int r, int c)> indices)
    {
        int col = 0;
        while (col < ncols)
        {
            var obstructors = Blocked(indices, row, col, element);
            if (obstructors.Count > 0)
                col = obstructors.Max(o => o.at.c + o.e.ColSpan);
            else if (col + element.ColSpan > ncols)
                return LayoutElement.AutoIndex;
            else
                return col;
        }
        return LayoutElement.AutoIndex;
    }

    public static (double W, double H) Perform(
        LayoutGrid grid, (double W, double H) minimum,
        (double W, double H) maximum, double scale = 1.0)
    {
        double minW = minimum.W, minH = minimum.H;
        double maxW = Math.Max(maximum.W, minW), maxH = Math.Max(maximum.H, minH);

        var rows = grid.Rows.Count > 0 ? new List<Track>(grid.Rows)
            : [new Track(Unit.Auto)];
        var cols = grid.Columns.Count > 0 ? new List<Track>(grid.Columns)
            : [new Track(Unit.Auto)];

        var indices = new Dictionary<LayoutElement, (int r, int c)>();
        foreach (var e in grid.Children)
            indices[e] = (e.Row >= 0 ? Math.Min(e.Row, rows.Count - 1) : e.Row,
                          e.Col >= 0 ? Math.Min(e.Col, cols.Count - 1) : e.Col);

        if (indices.Values.Any(v => v.r < 0 || v.c < 0))
        {
            var heights = new int[cols.Count];
            foreach (var e in indices.Keys.ToList())
            {
                var (r, c) = indices[e];
                bool isAuto = r == LayoutElement.AutoIndex
                    || c == LayoutElement.AutoIndex;
                if (r == LayoutElement.AutoIndex)
                {
                    if (c >= 0)
                    {
                        r = heights.Skip(c).Take(e.ColSpan)
                            .DefaultIfEmpty(0).Max();
                        var obst = Blocked(indices, r, c, e);
                        while (obst.Count > 0)
                        {
                            r = obst.Max(o => o.at.r + o.e.RowSpan);
                            obst = Blocked(indices, r, c, e);
                        }
                    }
                    else
                    {
                        r = heights.Min();
                    }
                }
                while (c == LayoutElement.AutoIndex)
                {
                    c = ColumnForRow(r, cols.Count, e, indices);
                    if (c > LayoutElement.AutoIndex) break;
                    var candidates = heights.Where(h => h > r).ToList();
                    r = candidates.Count > 0 ? candidates.Min() : rows.Count;
                }
                while (e.RowSpan + r > rows.Count)
                    rows.Add(new Track(Unit.Auto));
                indices[e] = (r, c);
                for (int i = 0; i < e.ColSpan && c + i < heights.Length; i++)
                {
                    int nxt = r + e.RowSpan;
                    if (isAuto || r == heights[c + i])
                    {
                        while (true)
                        {
                            var idxs = indices.Where(kv =>
                                kv.Value.c <= c + i
                                && kv.Value.c + kv.Key.ColSpan > c + i
                                && kv.Value.r <= nxt
                                && kv.Value.r + kv.Key.RowSpan > nxt).ToList();
                            if (idxs.Count == 0) break;
                            nxt = idxs.Max(kv => kv.Value.r + kv.Key.RowSpan);
                        }
                        heights[c + i] = nxt;
                    }
                }
            }
        }

        maxH *= scale;
        minH *= scale;
        var (pl, pt, pr, pb) = grid.Padding;
        double totalW = (double.IsInfinity(maxW) ? Big : maxW)
            - (pl + pr) * scale;
        double totalH = (double.IsInfinity(maxH) ? Big : maxH)
            - (pt + pb) * scale;
        double availW = totalW, availH = totalH;

        var rowSp = rows.Select(_ => new Space()).ToArray();
        var colSp = cols.Select(_ => new Space()).ToArray();
        var sizes = new Dictionary<LayoutElement, (double w, double h)>();

        int baseStarRow = -1;
        if (double.IsInfinity(maxH))
        {
            double weight = double.PositiveInfinity;
            for (int i = 0; i < rows.Count; i++)
                if (rows[i].Unit == Unit.Star && rows[i].Weight < weight)
                { weight = rows[i].Weight; baseStarRow = i; }
            if (baseStarRow >= 0)
                rows[baseStarRow] = new Track(Unit.Auto, weight);
        }
        int baseStarCol = -1;
        if (double.IsInfinity(maxW))
        {
            double weight = double.PositiveInfinity;
            for (int i = 0; i < cols.Count; i++)
                if (cols[i].Unit == Unit.Star && cols[i].Weight < weight)
                { weight = cols[i].Weight; baseStarCol = i; }
            if (baseStarCol >= 0)
                cols[baseStarCol] = new Track(Unit.Auto, weight);
        }

        for (int i = 0; i < rows.Count; i++)
            if (rows[i].Unit == Unit.Absolute)
            {
                availH -= rows[i].Weight * scale;
                rowSp[i].Size = rows[i].Weight * scale;
            }
        for (int i = 0; i < cols.Count; i++)
            if (cols[i].Unit == Unit.Absolute)
            {
                availW -= cols[i].Weight * scale;
                colSp[i].Size = cols[i].Weight * scale;
            }

        (double ml, double mt, double mr, double mb) Margins(LayoutElement e)
            => (e.Margin.L * scale, e.Margin.T * scale,
                e.Margin.R * scale, e.Margin.B * scale);

        foreach (var (e, (r, c)) in indices)
        {
            if (!e.Visible) { sizes[e] = (0, 0); continue; }
            double w = 0;
            bool inAutoCol = false, take = false;
            for (int i = c; i < Math.Min(cols.Count, c + e.ColSpan); i++)
            {
                if (cols[i].Unit == Unit.Absolute) w += cols[i].Weight;
                else if (!take) { take = true; w += availW; }
                if (cols[i].Unit == Unit.Auto) inAutoCol = true;
            }
            double h = 0;
            bool inAutoRow = false;
            take = false;
            for (int i = r; i < Math.Min(rows.Count, r + e.RowSpan); i++)
            {
                if (rows[i].Unit == Unit.Absolute) h += rows[i].Weight;
                else if (!take) { take = true; h += availH; }
                if (rows[i].Unit == Unit.Auto) inAutoRow = true;
            }
            if (!(inAutoCol || inAutoRow)) continue;
            var (ml, mt, mr, mb) = Margins(e);
            var (dw, dh) = e.Measure(Math.Max(0, w - ml - mr),
                                     Math.Max(0, h - mt - mb));
            sizes[e] = (dw, dh);
            int lastCol = Math.Min(cols.Count, c + e.ColSpan);
            if (c == lastCol - 1 && cols[c].Unit == Unit.Auto)
                colSp[c].Size = Math.Max(colSp[c].Size,
                                         Math.Max(dw + ml + mr, 0));
            int lastRow = Math.Min(rows.Count, r + e.RowSpan);
            if (r == lastRow - 1 && rows[r].Unit == Unit.Auto)
                rowSp[r].Size = Math.Max(rowSp[r].Size,
                                         Math.Max(dh + mt + mb, 0));
        }

        void SizeTracks(List<Track> tracks, Space[] spaces, double min,
                        double available, int baseStar, double padNear,
                        double padFar)
        {
            foreach (var (t, i) in tracks.Select((t, i) => (t, i)))
                if (t.Unit == Unit.Auto)
                {
                    if (spaces[i].Size > available)
                    {
                        spaces[i].Size = available;
                        available = 0;
                    }
                    else
                    {
                        spaces[i].Size = Math.Max(spaces[i].Size, 0);
                        available -= spaces[i].Size;
                    }
                }
            double weightSum = tracks.Where(t => t.Unit == Unit.Star)
                .Sum(t => t.Weight);
            double starUnit = weightSum > 0 ? available / weightSum : 0;
            if (baseStar >= 0)
            {
                weightSum += tracks[baseStar].Weight;
                starUnit = spaces[baseStar].Size * tracks[baseStar].Weight;
                double consumed = starUnit * weightSum;
                for (int i = 0; i < tracks.Count; i++)
                    if (i != baseStar && tracks[i].Unit != Unit.Star)
                        consumed += spaces[i].Size;
                double floor = min - (padNear + padFar) * scale;
                if (consumed < floor && weightSum > 0)
                {
                    starUnit += (floor - consumed) / weightSum;
                    spaces[baseStar].Size = starUnit * tracks[baseStar].Weight;
                }
            }
            for (int i = 0; i < tracks.Count; i++)
                if (tracks[i].Unit == Unit.Star)
                    spaces[i].Size = tracks[i].Weight * starUnit;
            double extra = min - spaces.Sum(s => s.Size)
                - (padNear + padFar) * scale;
            if (extra > 0)
                for (int i = tracks.Count - 1; i >= 0; i--)
                    if (tracks[i].Unit == Unit.Auto && spaces[i].Size > 0)
                    {
                        spaces[i].Size += extra;
                        break;
                    }
            spaces[0].Origin = padNear * scale;
            for (int i = 1; i < spaces.Length; i++)
                spaces[i].Origin = spaces[i - 1].Origin + spaces[i - 1].Size;
        }

        SizeTracks(cols, colSp, minW, availW, baseStarCol, pl, pr);
        SizeTracks(rows, rowSp, minH, availH, baseStarRow, pt, pb);

        foreach (var (e, (r, c)) in indices)
        {
            if (!e.Visible)
            {
                e.Place(colSp[c].Origin, rowSp[r].Origin, 0, 0);
                continue;
            }
            var (ml, mt, mr, mb) = Margins(e);
            double rowSize = 0, colSize = 0;
            for (int i = r; i < Math.Min(rows.Count, r + e.RowSpan); i++)
                rowSize += rowSp[i].Size;
            for (int i = c; i < Math.Min(cols.Count, c + e.ColSpan); i++)
                colSize += colSp[i].Size;
            double cw = Math.Max(colSize - ml - mr, 0);
            double ch = Math.Max(rowSize - mt - mb, 0);
            if (e.IsLabel)
                for (int i = r; i < Math.Min(rows.Count, r + e.RowSpan); i++)
                    if (rows[i].Unit == Unit.Auto) { ch = Big; break; }
            var (fw, fh) = e.Measure(cw, ch);
            double tw = fw + ml + mr, th = fh + mt + mb;
            var (dw, dh) = sizes.TryGetValue(e, out var s) ? s : (0, 0);
            double tdw = dw + ml + mr, tdh = dh + mt + mb;
            if (Math.Abs(tw - tdw) > 0.01)
            {
                int span = Math.Min(cols.Count - c, e.ColSpan);
                if (span == 1 && cols[c].Unit == Unit.Auto)
                {
                    bool others = sizes.Any(kv =>
                        !ReferenceEquals(kv.Key, e) && indices[kv.Key].c == c
                        && kv.Value.w + ml + mr >= colSp[c].Size);
                    if (tw > colSp[c].Size
                        || (tdw == colSp[c].Size && !others))
                    {
                        colSp[c].Size = tw;
                        SizeTracks(cols, colSp, minW, availW, baseStarCol,
                                   pl, pr);
                        tw = colSp[c].Size;
                    }
                }
            }
            if (Math.Abs(th - tdh) > 0.01)
            {
                int span = Math.Min(rows.Count - r, e.RowSpan);
                if (span == 1 && rows[r].Unit == Unit.Auto)
                {
                    bool others = sizes.Any(kv =>
                        !ReferenceEquals(kv.Key, e) && indices[kv.Key].r == r
                        && kv.Value.h + mt + mb >= rowSp[r].Size);
                    if (th > rowSp[r].Size
                        || (tdh == rowSp[r].Size && !others))
                    {
                        rowSp[r].Size = th;
                        SizeTracks(rows, rowSp, minH, availH, baseStarRow,
                                   pt, pb);
                        th = rowSp[r].Size;
                    }
                }
            }
            fw = tw - ml - mr;
            fh = th - mt - mb;
            double aw = Math.Max(colSize - ml - mr, 0);
            double ah = Math.Max(rowSize - mt - mb, 0);
            double x = colSp[c].Origin, y = rowSp[r].Origin;
            switch (e.VAlign)
            {
                case Align.Stretch: y = rowSp[r].Origin + mt; fh = ah; break;
                case Align.Near:
                    y = rowSp[r].Origin + mt; fh = Math.Min(fh, ah); break;
                case Align.Far:
                    y = rowSp[r].Origin + rowSize - (fh + mb);
                    fh = Math.Min(fh, ah); break;
                case Align.Center:
                    y = ah / 2.0 - fh / 2.0 + rowSp[r].Origin + mt;
                    fh = Math.Min(fh, ah); break;
            }
            switch (e.HAlign)
            {
                case Align.Stretch: x = colSp[c].Origin + ml; fw = aw; break;
                case Align.Near:
                    x = colSp[c].Origin + ml; fw = Math.Min(fw, aw); break;
                case Align.Far:
                    x = colSp[c].Origin + colSize - (fw + mr);
                    fw = Math.Min(fw, aw); break;
                case Align.Center:
                    x = aw / 2.0 - fw / 2.0 + colSp[c].Origin + ml;
                    fw = Math.Min(fw, colSize - ml - mr); break;
            }
            fw = Math.Max(fw, 0);
            fh = Math.Max(fh, 0);
            sizes[e] = (fw, fh);
            e.Place(x, y, fw, fh);
        }

        return (Math.Max(colSp.Sum(s => s.Size) + (pl + pr) * scale, minW),
                Math.Max(rowSp.Sum(s => s.Size) + (pt + pb) * scale, minH));
    }
}
