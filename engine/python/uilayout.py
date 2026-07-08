"""The abstract layout engine — iFactr's GridExtensions.PerformLayout,
ported faithfully (Samuel, 2026-07-08: "these are the abstract
invariants, not deprecated. These are how abstract layouts work").

Layout is computed ONCE in the abstract layer; a platform realizes
exactly two primitives per element — MEASURE (how big under these
constraints, its own text metrics) and PLACE (put it at the computed
rectangle, absolute positioning) — plus one scalar, the display scale.
That is why the same view renders identically on WPF, Compact, Android,
tk, or a framebuffer: the rows, columns, spans, margins, alignments,
auto-placement, star distribution, and the label-reflow second measure
all live HERE.

The port keeps the original's mechanics and order:
  1. default one auto row/column when none declared
  2. index clamping + AUTO-PLACEMENT (AutoLayoutIndex) via per-column
     high-water marks with obstruction walks (the flow layout)
  3. scale multiplies heights, margins, and padding
  4. under an infinite axis, the smallest-weight star becomes the BASE
     auto that sizes the other stars
  5. absolute tracks subtract first
  6. measure pass: only elements spanning autos measure; a single-span
     auto track takes the element's desired size as its high-water
  7. track sizing: autos clamp to the available (overflow truncates),
     stars split the remainder by weight, a minimum stretches the LAST
     non-zero auto, origins accumulate behind the padding
  8. arrange pass: spanned spaces sum, SECOND measure under the real
     constraints (a label in an auto row measures against infinite
     height — the reflow), single-span autos expand/shrink on a >0.01
     delta and re-size, alignment resolves the origin per axis
     (Stretch/Left|Top/Right|Bottom/Center), place(location, size)
  9. answers the grid's own size: track sums + padding, floored by min
"""

INF = float("inf")
AUTO_INDEX = -1                                  # Element.AutoLayoutIndex

# track unit types
ABSOLUTE, AUTO, STAR = "absolute", "auto", "star"

# alignments (per axis; Stretch is the iFactr default)
STRETCH, NEAR, FAR, CENTER = "stretch", "near", "far", "center"


class Track:
    """A Row or Column: a unit type and its weight (absolute size for
    ABSOLUTE, star weight for STAR, ignored for AUTO)."""

    def __init__(self, unit=AUTO, weight=1.0):
        self.unit = unit
        self.weight = float(weight)


class Element:
    """One abstract element: grid coordinates, spans, margins,
    alignments, visibility, and the platform's two primitives."""

    def __init__(self, measure, place, row=AUTO_INDEX, col=AUTO_INDEX,
                 row_span=1, col_span=1, margin=(0, 0, 0, 0),
                 halign=STRETCH, valign=STRETCH, visible=True,
                 is_label=False):
        self.measure = measure          # (w, h) -> (w, h)
        self.place = place              # (x, y, w, h) -> None
        self.row = row
        self.col = col
        self.row_span = max(1, row_span)
        self.col_span = max(1, col_span)
        self.margin = margin            # left, top, right, bottom
        self.halign = halign
        self.valign = valign
        self.visible = visible
        self.is_label = is_label        # the reflow rule


class Grid:
    def __init__(self, rows=None, columns=None, padding=(0, 0, 0, 0)):
        self.rows = list(rows or [])
        self.columns = list(columns or [])
        self.padding = padding
        self.children = []

    def add(self, element):
        self.children.append(element)
        return element


class _Space:
    __slots__ = ("origin", "size")

    def __init__(self):
        self.origin = 0.0
        self.size = 0.0


def _blocked(indices, row, col, element):
    """The controls obstructing (row, col) for `element`'s spans."""
    out = []
    for e, (r, c) in indices.items():
        if e is element:
            continue
        if (c <= col < c + e.col_span or col <= c < col + element.col_span) \
                and (r <= row < r + e.row_span
                     or row <= r < row + element.row_span):
            if c <= col + element.col_span - 1 and c + e.col_span > col \
                    and r <= row + element.row_span - 1 and r + e.row_span > row:
                out.append((e, (r, c)))
    return out


def _column_for_row(row, ncols, element, indices):
    col = 0
    while col < ncols:
        obstructors = _blocked(indices, row, col, element)
        if obstructors:
            col = max(c + e.col_span for e, (_r, c) in obstructors)
        elif col + element.col_span > ncols:
            return AUTO_INDEX
        else:
            return col
    return AUTO_INDEX


def perform_layout(grid, minimum, maximum, scale=1.0):
    """The engine. minimum/maximum are (w, h); INF allowed in maximum
    (a star on an infinite axis promotes the base-star rule). Answers
    the grid's laid-out size."""
    min_w, min_h = float(minimum[0]), float(minimum[1])
    max_w, max_h = float(maximum[0]), float(maximum[1])
    if min_w == INF or min_h == INF:
        raise ValueError("minimum size must be finite")
    max_w = max(max_w, min_w)
    max_h = max(max_h, min_h)

    rows = list(grid.rows) or [Track(AUTO)]
    cols = list(grid.columns) or [Track(AUTO)]

    # -- index clamping --
    indices = {}
    for e in grid.children:
        indices[e] = (min(e.row, len(rows) - 1) if e.row >= 0 else e.row,
                      min(e.col, len(cols) - 1) if e.col >= 0 else e.col)

    # -- auto placement (the flow layout) --
    if any(r < 0 or c < 0 for (r, c) in indices.values()):
        heights = [0] * len(cols)
        for e in list(indices):
            r, c = indices[e]
            is_auto = r == AUTO_INDEX or c == AUTO_INDEX
            if r == AUTO_INDEX:
                if c >= 0:
                    r = max(heights[c:c + e.col_span] or [0])
                    obst = _blocked(indices, r, c, e)
                    while obst:
                        r = max(rr + oe.row_span for oe, (rr, _c) in obst)
                        obst = _blocked(indices, r, c, e)
                else:
                    r = min(heights)
            while c == AUTO_INDEX:
                c = _column_for_row(r, len(cols), e, indices)
                if c > AUTO_INDEX:
                    break
                candidates = [h for h in heights if h > r]
                r = min(candidates) if candidates else len(rows)
            while e.row_span + r > len(rows):
                rows.append(Track(AUTO))
            indices[e] = (r, c)
            for i in range(e.col_span):
                if c + i >= len(heights):
                    break
                nxt = r + e.row_span
                if is_auto or r == heights[c + i]:
                    while True:
                        idxs = [(oe, rc) for oe, rc in indices.items()
                                if rc[1] <= c + i < rc[1] + oe.col_span
                                and rc[0] <= nxt < rc[0] + oe.row_span]
                        if not idxs:
                            break
                        nxt = max(rr + oe.row_span for oe, (rr, _c) in idxs)
                    heights[c + i] = nxt

    # -- scale (heights, margins, padding are scaled; widths are not,
    #    matching the original) --
    max_h *= scale
    min_h *= scale
    pl, pt, pr, pb = grid.padding
    total_w = (1e308 if max_w == INF else max_w) - (pl + pr) * scale
    total_h = (1e308 if max_h == INF else max_h) - (pt + pb) * scale
    avail_w, avail_h = total_w, total_h

    row_sp = [_Space() for _ in rows]
    col_sp = [_Space() for _ in cols]
    sizes = {}

    # -- base star under infinity --
    base_star_row = -1
    if max_h == INF:
        weight = INF
        for i, r in enumerate(rows):
            if r.unit == STAR and r.weight < weight:
                weight, base_star_row = r.weight, i
        if base_star_row >= 0:
            rows[base_star_row] = Track(AUTO, weight)
    base_star_col = -1
    if max_w == INF:
        weight = INF
        for i, c in enumerate(cols):
            if c.unit == STAR and c.weight < weight:
                weight, base_star_col = c.weight, i
        if base_star_col >= 0:
            cols[base_star_col] = Track(AUTO, weight)

    # -- absolutes subtract first --
    for i, r in enumerate(rows):
        if r.unit == ABSOLUTE:
            avail_h -= r.weight * scale
            row_sp[i].size = r.weight * scale
    for i, c in enumerate(cols):
        if c.unit == ABSOLUTE:
            avail_w -= c.weight * scale
            col_sp[i].size = c.weight * scale

    def margins(e):
        ml, mt, mr, mb = e.margin
        return ml * scale, mt * scale, mr * scale, mb * scale

    # -- the measure pass --
    for e, (r, c) in indices.items():
        if not e.visible:
            sizes[e] = (0.0, 0.0)
            continue
        w = 0.0
        in_auto_col = take = False
        for i in range(c, min(len(cols), c + e.col_span)):
            if cols[i].unit == ABSOLUTE:
                w += cols[i].weight
            elif not take:
                take = True
                w += avail_w
            if cols[i].unit == AUTO:
                in_auto_col = True
        h = 0.0
        in_auto_row = take = False
        for i in range(r, min(len(rows), r + e.row_span)):
            if rows[i].unit == ABSOLUTE:
                h += rows[i].weight
            elif not take:
                take = True
                h += avail_h
            if rows[i].unit == AUTO:
                in_auto_row = True
        if not (in_auto_col or in_auto_row):
            continue
        ml, mt, mr, mb = margins(e)
        dw, dh = e.measure(max(0.0, w - ml - mr), max(0.0, h - mt - mb))
        sizes[e] = (dw, dh)
        last_col = min(len(cols), c + e.col_span)
        if c == last_col - 1 and cols[c].unit == AUTO:
            col_sp[c].size = max(col_sp[c].size, max(dw + ml + mr, 0.0))
        last_row = min(len(rows), r + e.row_span)
        if r == last_row - 1 and rows[r].unit == AUTO:
            row_sp[r].size = max(row_sp[r].size, max(dh + mt + mb, 0.0))

    def size_tracks(tracks, spaces, minimum, available, base_star,
                    pad_near, pad_far):
        for i, t in enumerate(tracks):
            if t.unit == AUTO:
                if spaces[i].size > available:
                    spaces[i].size = available
                    available = 0.0
                else:
                    spaces[i].size = max(spaces[i].size, 0.0)
                    available -= spaces[i].size
        weight_sum = sum(t.weight for t in tracks if t.unit == STAR)
        star_unit = available / weight_sum if weight_sum else 0.0
        if base_star >= 0:
            weight_sum += tracks[base_star].weight
            star_unit = spaces[base_star].size * tracks[base_star].weight
            consumed = star_unit * weight_sum
            for i, t in enumerate(tracks):
                if i != base_star and t.unit != STAR:
                    consumed += spaces[i].size
            floor = minimum - (pad_near + pad_far) * scale
            if consumed < floor and weight_sum:
                star_unit += (floor - consumed) / weight_sum
                spaces[base_star].size = (star_unit
                                          * tracks[base_star].weight)
        for i, t in enumerate(tracks):
            if t.unit == STAR:
                spaces[i].size = t.weight * star_unit
        extra = (minimum - sum(s.size for s in spaces)
                 - (pad_near + pad_far) * scale)
        if extra > 0:
            for i in range(len(tracks) - 1, -1, -1):
                if tracks[i].unit == AUTO and spaces[i].size > 0:
                    spaces[i].size += extra
                    break
        spaces[0].origin = pad_near * scale
        for i in range(1, len(spaces)):
            spaces[i].origin = spaces[i - 1].origin + spaces[i - 1].size

    size_tracks(cols, col_sp, min_w, avail_w, base_star_col, pl, pr)
    size_tracks(rows, row_sp, min_h, avail_h, base_star_row, pt, pb)

    # -- the arrange pass --
    for e, (r, c) in indices.items():
        if not e.visible:
            e.place(col_sp[c].origin, row_sp[r].origin, 0.0, 0.0)
            continue
        ml, mt, mr, mb = margins(e)
        row_size = sum(row_sp[i].size
                       for i in range(r, min(len(rows), r + e.row_span)))
        col_size = sum(col_sp[i].size
                       for i in range(c, min(len(cols), c + e.col_span)))
        cw = max(col_size - ml - mr, 0.0)
        ch = max(row_size - mt - mb, 0.0)
        if e.is_label and any(
                rows[i].unit == AUTO
                for i in range(r, min(len(rows), r + e.row_span))):
            ch = 1e308                        # the label reflow rule
        fw, fh = e.measure(cw, ch)
        tw, th = fw + ml + mr, fh + mt + mb
        dw, dh = sizes.get(e, (0.0, 0.0))
        tdw, tdh = dw + ml + mr, dh + mt + mb
        if abs(tw - tdw) > 0.01:
            span = min(len(cols) - c, e.col_span)
            if span == 1 and cols[c].unit == AUTO:
                others = any(
                    ee is not e and indices[ee][1] == c
                    and sizes.get(ee, (0, 0))[0] + ml + mr >= col_sp[c].size
                    for ee in sizes)
                if tw > col_sp[c].size or (tdw == col_sp[c].size
                                           and not others):
                    col_sp[c].size = tw
                    size_tracks(cols, col_sp, min_w, avail_w,
                                base_star_col, pl, pr)
                    tw = col_sp[c].size
        if abs(th - tdh) > 0.01:
            span = min(len(rows) - r, e.row_span)
            if span == 1 and rows[r].unit == AUTO:
                others = any(
                    ee is not e and indices[ee][0] == r
                    and sizes.get(ee, (0, 0))[1] + mt + mb >= row_sp[r].size
                    for ee in sizes)
                if th > row_sp[r].size or (tdh == row_sp[r].size
                                           and not others):
                    row_sp[r].size = th
                    size_tracks(rows, row_sp, min_h, avail_h,
                                base_star_row, pt, pb)
                    th = row_sp[r].size
        fw, fh = tw - ml - mr, th - mt - mb
        aw = max(col_size - ml - mr, 0.0)
        ah = max(row_size - mt - mb, 0.0)
        x, y = col_sp[c].origin, row_sp[r].origin
        if e.valign == STRETCH:
            y = row_sp[r].origin + mt
            fh = ah
        elif e.valign == NEAR:
            y = row_sp[r].origin + mt
            fh = min(fh, ah)
        elif e.valign == FAR:
            y = row_sp[r].origin + row_size - (fh + mb)
            fh = min(fh, ah)
        elif e.valign == CENTER:
            y = ah / 2.0 - fh / 2.0 + row_sp[r].origin + mt
            fh = min(fh, ah)
        if e.halign == STRETCH:
            x = col_sp[c].origin + ml
            fw = aw
        elif e.halign == NEAR:
            x = col_sp[c].origin + ml
            fw = min(fw, aw)
        elif e.halign == FAR:
            x = col_sp[c].origin + col_size - (fw + mr)
            fw = min(fw, aw)
        elif e.halign == CENTER:
            x = aw / 2.0 - fw / 2.0 + col_sp[c].origin + ml
            fw = min(fw, col_size - ml - mr)
        fw, fh = max(fw, 0.0), max(fh, 0.0)
        sizes[e] = (fw, fh)
        e.place(x, y, fw, fh)

    return (max(sum(s.size for s in col_sp) + (pl + pr) * scale, min_w),
            max(sum(s.size for s in row_sp) + (pt + pb) * scale, min_h))
