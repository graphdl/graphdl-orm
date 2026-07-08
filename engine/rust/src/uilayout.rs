// The abstract layout engine — the RUST transliteration of
// pyarest.uilayout / csharp Layout.cs (iFactr's
// GridExtensions.PerformLayout, the abstract invariant: layout
// computes ONCE here; a platform realizes exactly MEASURE and PLACE
// plus one scalar, the display scale). The third certified-equal
// engine — the same seven golden cases gate all three — and the
// foundation of the Slint/UEFI target (the Compact blueprint: one
// metric primitive, absolute placement, layout at paint time).
#![allow(dead_code)]

pub const AUTO_INDEX: i32 = -1;
const BIG: f64 = 1e308;

#[derive(Clone, Copy, PartialEq)]
pub enum Unit {
    Absolute,
    Auto,
    Star,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Stretch,
    Near,
    Far,
    Center,
}

#[derive(Clone, Copy)]
pub struct Track {
    pub unit: Unit,
    pub weight: f64,
}

impl Track {
    pub fn new(unit: Unit, weight: f64) -> Self {
        Track { unit, weight }
    }
    pub fn auto() -> Self {
        Track::new(Unit::Auto, 1.0)
    }
}

pub struct LayoutElement<'a> {
    pub measure: Box<dyn FnMut(f64, f64) -> (f64, f64) + 'a>,
    pub place: Box<dyn FnMut(f64, f64, f64, f64) + 'a>,
    pub row: i32,
    pub col: i32,
    pub row_span: usize,
    pub col_span: usize,
    pub margin: (f64, f64, f64, f64),
    pub halign: Align,
    pub valign: Align,
    pub visible: bool,
    pub is_label: bool,
}

impl<'a> LayoutElement<'a> {
    pub fn new(
        measure: impl FnMut(f64, f64) -> (f64, f64) + 'a,
        place: impl FnMut(f64, f64, f64, f64) + 'a,
    ) -> Self {
        LayoutElement {
            measure: Box::new(measure),
            place: Box::new(place),
            row: AUTO_INDEX,
            col: AUTO_INDEX,
            row_span: 1,
            col_span: 1,
            margin: (0.0, 0.0, 0.0, 0.0),
            halign: Align::Stretch,
            valign: Align::Stretch,
            visible: true,
            is_label: false,
        }
    }
}

pub struct LayoutGrid<'a> {
    pub rows: Vec<Track>,
    pub columns: Vec<Track>,
    pub padding: (f64, f64, f64, f64),
    pub children: Vec<LayoutElement<'a>>,
}

impl<'a> LayoutGrid<'a> {
    pub fn new() -> Self {
        LayoutGrid {
            rows: Vec::new(),
            columns: Vec::new(),
            padding: (0.0, 0.0, 0.0, 0.0),
            children: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Space {
    origin: f64,
    size: f64,
}

fn blocked(
    indices: &[(usize, i32, i32)],
    spans: &[(usize, usize)],
    row: i32,
    col: i32,
    me: usize,
) -> Vec<(usize, i32, i32)> {
    let (mrs, mcs) = spans[me];
    indices
        .iter()
        .filter(|&&(e, r, c)| {
            if e == me {
                return false;
            }
            let (ers, ecs) = spans[e];
            c <= col + mcs as i32 - 1
                && c + ecs as i32 > col
                && r <= row + mrs as i32 - 1
                && r + ers as i32 > row
        })
        .copied()
        .collect()
}

fn column_for_row(
    row: i32,
    ncols: usize,
    me: usize,
    indices: &[(usize, i32, i32)],
    spans: &[(usize, usize)],
) -> i32 {
    let mut col: i32 = 0;
    while (col as usize) < ncols {
        let obst = blocked(indices, spans, row, col, me);
        if !obst.is_empty() {
            col = obst
                .iter()
                .map(|&(e, _r, c)| c + spans[e].1 as i32)
                .max()
                .unwrap();
        } else if col as usize + spans[me].1 > ncols {
            return AUTO_INDEX;
        } else {
            return col;
        }
    }
    AUTO_INDEX
}

pub fn perform_layout(
    grid: &mut LayoutGrid,
    minimum: (f64, f64),
    maximum: (f64, f64),
    scale: f64,
) -> (f64, f64) {
    let (min_w, mut min_h) = minimum;
    let (mut max_w, mut max_h) = (maximum.0.max(min_w), maximum.1.max(min_h));
    let _ = &mut max_w;

    let mut rows = if grid.rows.is_empty() {
        vec![Track::auto()]
    } else {
        grid.rows.clone()
    };
    let mut cols = if grid.columns.is_empty() {
        vec![Track::auto()]
    } else {
        grid.columns.clone()
    };

    let n = grid.children.len();
    let spans: Vec<(usize, usize)> = grid
        .children
        .iter()
        .map(|e| (e.row_span.max(1), e.col_span.max(1)))
        .collect();
    // (child index, row, col)
    let mut indices: Vec<(usize, i32, i32)> = grid
        .children
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i,
                if e.row >= 0 {
                    e.row.min(rows.len() as i32 - 1)
                } else {
                    e.row
                },
                if e.col >= 0 {
                    e.col.min(cols.len() as i32 - 1)
                } else {
                    e.col
                },
            )
        })
        .collect();

    // -- auto placement (the flow layout) --
    if indices.iter().any(|&(_i, r, c)| r < 0 || c < 0) {
        let mut heights = vec![0i32; cols.len()];
        for k in 0..n {
            let (_i, mut r, mut c) = indices[k];
            let is_auto = r == AUTO_INDEX || c == AUTO_INDEX;
            if r == AUTO_INDEX {
                if c >= 0 {
                    r = heights[c as usize..(c as usize + spans[k].1).min(heights.len())]
                        .iter()
                        .copied()
                        .max()
                        .unwrap_or(0);
                    let mut obst = blocked(&indices, &spans, r, c, k);
                    while !obst.is_empty() {
                        r = obst
                            .iter()
                            .map(|&(e, rr, _c)| rr + spans[e].0 as i32)
                            .max()
                            .unwrap();
                        obst = blocked(&indices, &spans, r, c, k);
                    }
                } else {
                    r = heights.iter().copied().min().unwrap_or(0);
                }
            }
            while c == AUTO_INDEX {
                c = column_for_row(r, cols.len(), k, &indices, &spans);
                if c > AUTO_INDEX {
                    break;
                }
                let candidates: Vec<i32> =
                    heights.iter().copied().filter(|&h| h > r).collect();
                r = candidates
                    .iter()
                    .copied()
                    .min()
                    .unwrap_or(rows.len() as i32);
            }
            while spans[k].0 as i32 + r > rows.len() as i32 {
                rows.push(Track::auto());
            }
            indices[k] = (k, r, c);
            for i in 0..spans[k].1 {
                if c as usize + i >= heights.len() {
                    break;
                }
                let mut nxt = r + spans[k].0 as i32;
                if is_auto || r == heights[c as usize + i] {
                    loop {
                        let idxs: Vec<(usize, i32, i32)> = indices
                            .iter()
                            .filter(|&&(e, rr, cc)| {
                                cc <= (c + i as i32)
                                    && cc + spans[e].1 as i32 > (c + i as i32)
                                    && rr <= nxt
                                    && rr + spans[e].0 as i32 > nxt
                            })
                            .copied()
                            .collect();
                        if idxs.is_empty() {
                            break;
                        }
                        nxt = idxs
                            .iter()
                            .map(|&(e, rr, _c)| rr + spans[e].0 as i32)
                            .max()
                            .unwrap();
                    }
                    heights[c as usize + i] = nxt;
                }
            }
        }
    }

    max_h *= scale;
    min_h *= scale;
    let (pl, pt, pr, pb) = grid.padding;
    let total_w = if max_w.is_infinite() { BIG } else { max_w } - (pl + pr) * scale;
    let total_h = if max_h.is_infinite() { BIG } else { max_h } - (pt + pb) * scale;
    let mut avail_w = total_w;
    let mut avail_h = total_h;

    let mut row_sp = vec![Space::default(); rows.len()];
    let mut col_sp = vec![Space::default(); cols.len()];
    let mut sizes: Vec<Option<(f64, f64)>> = vec![None; n];

    // -- base star under infinity --
    let mut base_star_row: i32 = -1;
    if max_h.is_infinite() {
        let mut weight = f64::INFINITY;
        for (i, r) in rows.iter().enumerate() {
            if r.unit == Unit::Star && r.weight < weight {
                weight = r.weight;
                base_star_row = i as i32;
            }
        }
        if base_star_row >= 0 {
            rows[base_star_row as usize] = Track::new(Unit::Auto, weight);
        }
    }
    let mut base_star_col: i32 = -1;
    if max_w.is_infinite() {
        let mut weight = f64::INFINITY;
        for (i, c) in cols.iter().enumerate() {
            if c.unit == Unit::Star && c.weight < weight {
                weight = c.weight;
                base_star_col = i as i32;
            }
        }
        if base_star_col >= 0 {
            cols[base_star_col as usize] = Track::new(Unit::Auto, weight);
        }
    }

    for (i, r) in rows.iter().enumerate() {
        if r.unit == Unit::Absolute {
            avail_h -= r.weight * scale;
            row_sp[i].size = r.weight * scale;
        }
    }
    for (i, c) in cols.iter().enumerate() {
        if c.unit == Unit::Absolute {
            avail_w -= c.weight * scale;
            col_sp[i].size = c.weight * scale;
        }
    }

    // -- the measure pass --
    for k in 0..n {
        let (_i, r, c) = indices[k];
        if !grid.children[k].visible {
            sizes[k] = Some((0.0, 0.0));
            continue;
        }
        let (mut w, mut h) = (0.0f64, 0.0f64);
        let (mut in_auto_col, mut take) = (false, false);
        let last_col = cols.len().min(c as usize + spans[k].1);
        for i in c as usize..last_col {
            match cols[i].unit {
                Unit::Absolute => w += cols[i].weight,
                _ if !take => {
                    take = true;
                    w += avail_w;
                }
                _ => {}
            }
            if cols[i].unit == Unit::Auto {
                in_auto_col = true;
            }
        }
        take = false;
        let mut in_auto_row = false;
        let last_row = rows.len().min(r as usize + spans[k].0);
        for i in r as usize..last_row {
            match rows[i].unit {
                Unit::Absolute => h += rows[i].weight,
                _ if !take => {
                    take = true;
                    h += avail_h;
                }
                _ => {}
            }
            if rows[i].unit == Unit::Auto {
                in_auto_row = true;
            }
        }
        if !(in_auto_col || in_auto_row) {
            continue;
        }
        let (ml, mt, mr, mb) = grid.children[k].margin;
        let (ml, mt, mr, mb) = (ml * scale, mt * scale, mr * scale, mb * scale);
        let (dw, dh) = (grid.children[k].measure)(
            (w - ml - mr).max(0.0),
            (h - mt - mb).max(0.0),
        );
        sizes[k] = Some((dw, dh));
        if c as usize == last_col - 1 && cols[c as usize].unit == Unit::Auto {
            col_sp[c as usize].size =
                col_sp[c as usize].size.max((dw + ml + mr).max(0.0));
        }
        if r as usize == last_row - 1 && rows[r as usize].unit == Unit::Auto {
            row_sp[r as usize].size =
                row_sp[r as usize].size.max((dh + mt + mb).max(0.0));
        }
    }

    fn size_tracks(
        tracks: &[Track],
        spaces: &mut [Space],
        minimum: f64,
        mut available: f64,
        base_star: i32,
        pad_near: f64,
        pad_far: f64,
        scale: f64,
    ) {
        for (i, t) in tracks.iter().enumerate() {
            if t.unit == Unit::Auto {
                if spaces[i].size > available {
                    spaces[i].size = available;
                    available = 0.0;
                } else {
                    spaces[i].size = spaces[i].size.max(0.0);
                    available -= spaces[i].size;
                }
            }
        }
        let mut weight_sum: f64 = tracks
            .iter()
            .filter(|t| t.unit == Unit::Star)
            .map(|t| t.weight)
            .sum();
        let mut star_unit = if weight_sum > 0.0 {
            available / weight_sum
        } else {
            0.0
        };
        if base_star >= 0 {
            let b = base_star as usize;
            weight_sum += tracks[b].weight;
            star_unit = spaces[b].size * tracks[b].weight;
            let mut consumed = star_unit * weight_sum;
            for (i, t) in tracks.iter().enumerate() {
                if i != b && t.unit != Unit::Star {
                    consumed += spaces[i].size;
                }
            }
            let floor = minimum - (pad_near + pad_far) * scale;
            if consumed < floor && weight_sum > 0.0 {
                star_unit += (floor - consumed) / weight_sum;
                spaces[b].size = star_unit * tracks[b].weight;
            }
        }
        for (i, t) in tracks.iter().enumerate() {
            if t.unit == Unit::Star {
                spaces[i].size = t.weight * star_unit;
            }
        }
        let extra = minimum
            - spaces.iter().map(|s| s.size).sum::<f64>()
            - (pad_near + pad_far) * scale;
        if extra > 0.0 {
            for i in (0..tracks.len()).rev() {
                if tracks[i].unit == Unit::Auto && spaces[i].size > 0.0 {
                    spaces[i].size += extra;
                    break;
                }
            }
        }
        spaces[0].origin = pad_near * scale;
        for i in 1..spaces.len() {
            spaces[i].origin = spaces[i - 1].origin + spaces[i - 1].size;
        }
    }

    size_tracks(&cols, &mut col_sp, min_w, avail_w, base_star_col, pl, pr, scale);
    size_tracks(&rows, &mut row_sp, min_h, avail_h, base_star_row, pt, pb, scale);

    // -- the arrange pass --
    for k in 0..n {
        let (_i, r, c) = indices[k];
        let (r, c) = (r as usize, c as usize);
        if !grid.children[k].visible {
            let (x, y) = (col_sp[c].origin, row_sp[r].origin);
            (grid.children[k].place)(x, y, 0.0, 0.0);
            continue;
        }
        let (ml, mt, mr, mb) = grid.children[k].margin;
        let (ml, mt, mr, mb) = (ml * scale, mt * scale, mr * scale, mb * scale);
        let row_size: f64 = (r..rows.len().min(r + spans[k].0))
            .map(|i| row_sp[i].size)
            .sum();
        let col_size: f64 = (c..cols.len().min(c + spans[k].1))
            .map(|i| col_sp[i].size)
            .sum();
        let cw = (col_size - ml - mr).max(0.0);
        let mut ch = (row_size - mt - mb).max(0.0);
        if grid.children[k].is_label
            && (r..rows.len().min(r + spans[k].0))
                .any(|i| rows[i].unit == Unit::Auto)
        {
            ch = BIG; // the label reflow rule
        }
        let (fw0, fh0) = (grid.children[k].measure)(cw, ch);
        let (mut tw, mut th) = (fw0 + ml + mr, fh0 + mt + mb);
        let (dw, dh) = sizes[k].unwrap_or((0.0, 0.0));
        let (tdw, tdh) = (dw + ml + mr, dh + mt + mb);
        if (tw - tdw).abs() > 0.01 {
            let span = (cols.len() - c).min(spans[k].1);
            if span == 1 && cols[c].unit == Unit::Auto {
                let others = (0..n).any(|e| {
                    e != k
                        && indices[e].2 as usize == c
                        && sizes[e].map(|s| s.0).unwrap_or(0.0) + ml + mr
                            >= col_sp[c].size
                });
                if tw > col_sp[c].size || (tdw == col_sp[c].size && !others) {
                    col_sp[c].size = tw;
                    size_tracks(
                        &cols, &mut col_sp, min_w, avail_w, base_star_col,
                        pl, pr, scale,
                    );
                    tw = col_sp[c].size;
                }
            }
        }
        if (th - tdh).abs() > 0.01 {
            let span = (rows.len() - r).min(spans[k].0);
            if span == 1 && rows[r].unit == Unit::Auto {
                let others = (0..n).any(|e| {
                    e != k
                        && indices[e].1 as usize == r
                        && sizes[e].map(|s| s.1).unwrap_or(0.0) + mt + mb
                            >= row_sp[r].size
                });
                if th > row_sp[r].size || (tdh == row_sp[r].size && !others) {
                    row_sp[r].size = th;
                    size_tracks(
                        &rows, &mut row_sp, min_h, avail_h, base_star_row,
                        pt, pb, scale,
                    );
                    th = row_sp[r].size;
                }
            }
        }
        let (mut fw, mut fh) = (tw - ml - mr, th - mt - mb);
        let aw = (col_size - ml - mr).max(0.0);
        let ah = (row_size - mt - mb).max(0.0);
        let (mut x, mut y) = (col_sp[c].origin, row_sp[r].origin);
        match grid.children[k].valign {
            Align::Stretch => {
                y = row_sp[r].origin + mt;
                fh = ah;
            }
            Align::Near => {
                y = row_sp[r].origin + mt;
                fh = fh.min(ah);
            }
            Align::Far => {
                y = row_sp[r].origin + row_size - (fh + mb);
                fh = fh.min(ah);
            }
            Align::Center => {
                y = ah / 2.0 - fh / 2.0 + row_sp[r].origin + mt;
                fh = fh.min(ah);
            }
        }
        match grid.children[k].halign {
            Align::Stretch => {
                x = col_sp[c].origin + ml;
                fw = aw;
            }
            Align::Near => {
                x = col_sp[c].origin + ml;
                fw = fw.min(aw);
            }
            Align::Far => {
                x = col_sp[c].origin + col_size - (fw + mr);
                fw = fw.min(aw);
            }
            Align::Center => {
                x = aw / 2.0 - fw / 2.0 + col_sp[c].origin + ml;
                fw = fw.min(col_size - ml - mr);
            }
        }
        fw = fw.max(0.0);
        fh = fh.max(0.0);
        sizes[k] = Some((fw, fh));
        (grid.children[k].place)(x, y, fw, fh);
    }

    (
        (col_sp.iter().map(|s| s.size).sum::<f64>() + (pl + pr) * scale)
            .max(min_w),
        (row_sp.iter().map(|s| s.size).sum::<f64>() + (pt + pb) * scale)
            .max(min_h),
    )
}

// ---- the golden gate: the SAME seven cases as tests/test_uilayout.py
// and arest-show --layout-selftest — three engines, one meaning ----
#[cfg(test)]
mod golden {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn boxed<'a>(
        w: f64,
        h: f64,
        rect: Rc<RefCell<(f64, f64, f64, f64)>>,
        wrap: bool,
    ) -> LayoutElement<'a> {
        LayoutElement::new(
            move |cw, ch| {
                if wrap {
                    let ww = w.min(cw);
                    (ww, (h * (w / ww.max(1.0))).min(ch))
                } else {
                    (w.min(cw), h.min(ch))
                }
            },
            move |x, y, pw, ph| {
                *rect.borrow_mut() = (x, y, pw, ph);
            },
        )
    }

    fn rect() -> Rc<RefCell<(f64, f64, f64, f64)>> {
        Rc::new(RefCell::new((0.0, 0.0, 0.0, 0.0)))
    }

    fn eq(r: (f64, f64, f64, f64), x: f64, y: f64, w: f64, h: f64) -> bool {
        (r.0 - x).abs() < 0.01
            && (r.1 - y).abs() < 0.01
            && (r.2 - w).abs() < 0.01
            && (r.3 - h).abs() < 0.01
    }

    #[test]
    fn the_seven_golden_cases_hold() {
        let inf = f64::INFINITY;

        // 1. label column + star value column
        {
            let (k1r, v1r, k2r, v2r) = (rect(), rect(), rect(), rect());
            let mut g = LayoutGrid::new();
            g.columns = vec![Track::auto(), Track::new(Unit::Star, 1.0)];
            g.rows = vec![Track::auto(), Track::auto()];
            let mut k1 = boxed(40.0, 20.0, k1r.clone(), false);
            k1.row = 0; k1.col = 0; k1.halign = Align::Near; k1.valign = Align::Near;
            let mut v1 = boxed(500.0, 20.0, v1r.clone(), false);
            v1.row = 0; v1.col = 1;
            let mut k2 = boxed(60.0, 20.0, k2r.clone(), false);
            k2.row = 1; k2.col = 0; k2.halign = Align::Near; k2.valign = Align::Near;
            let mut v2 = boxed(30.0, 20.0, v2r.clone(), false);
            v2.row = 1; v2.col = 1; v2.halign = Align::Near; v2.valign = Align::Near;
            g.children = vec![k1, v1, k2, v2];
            let (w, h) = perform_layout(&mut g, (0.0, 0.0), (300.0, inf), 1.0);
            assert_eq!((w, h), (300.0, 40.0));
            assert!(eq(*k1r.borrow(), 0.0, 0.0, 40.0, 20.0));
            assert!(eq(*k2r.borrow(), 0.0, 20.0, 60.0, 20.0));
            assert!(eq(*v1r.borrow(), 60.0, 0.0, 240.0, 20.0));
            assert!(v2r.borrow().0 == 60.0 && v2r.borrow().2 == 30.0);
        }

        // 2. absolute + weighted stars + padding
        {
            let (ar, br, cr) = (rect(), rect(), rect());
            let mut g = LayoutGrid::new();
            g.columns = vec![
                Track::new(Unit::Absolute, 50.0),
                Track::new(Unit::Star, 1.0),
                Track::new(Unit::Star, 3.0),
            ];
            g.rows = vec![Track::new(Unit::Absolute, 30.0)];
            g.padding = (10.0, 5.0, 10.0, 5.0);
            for (i, r) in [(0, &ar), (1, &br), (2, &cr)] {
                let mut b = boxed(999.0, 999.0, r.clone(), false);
                b.row = 0; b.col = i;
                g.children.push(b);
            }
            perform_layout(&mut g, (0.0, 0.0), (270.0, 40.0), 1.0);
            assert!(eq(*ar.borrow(), 10.0, 5.0, 50.0, 30.0));
            assert!(eq(*br.borrow(), 60.0, 5.0, 50.0, 30.0));
            assert!(eq(*cr.borrow(), 110.0, 5.0, 150.0, 30.0));
        }

        // 3. margins + alignments
        {
            let (nr, fr, cr2, sr) = (rect(), rect(), rect(), rect());
            let mut g = LayoutGrid::new();
            g.columns = vec![Track::new(Unit::Absolute, 100.0)];
            g.rows = vec![Track::new(Unit::Absolute, 100.0)];
            let mut near = boxed(20.0, 10.0, nr.clone(), false);
            near.row = 0; near.col = 0; near.halign = Align::Near;
            near.valign = Align::Near; near.margin = (5.0, 6.0, 0.0, 0.0);
            let mut far = boxed(20.0, 10.0, fr.clone(), false);
            far.row = 0; far.col = 0; far.halign = Align::Far;
            far.valign = Align::Far; far.margin = (0.0, 0.0, 5.0, 6.0);
            let mut ctr = boxed(20.0, 10.0, cr2.clone(), false);
            ctr.row = 0; ctr.col = 0; ctr.halign = Align::Center;
            ctr.valign = Align::Center;
            let mut fill = boxed(20.0, 10.0, sr.clone(), false);
            fill.row = 0; fill.col = 0;
            g.children = vec![near, far, ctr, fill];
            perform_layout(&mut g, (0.0, 0.0), (100.0, 100.0), 1.0);
            assert!(eq(*nr.borrow(), 5.0, 6.0, 20.0, 10.0));
            assert!(eq(*fr.borrow(), 75.0, 84.0, 20.0, 10.0));
            assert!(eq(*cr2.borrow(), 40.0, 45.0, 20.0, 10.0));
            assert!(eq(*sr.borrow(), 0.0, 0.0, 100.0, 100.0));
        }

        // 4. auto placement flows rows
        {
            let rs: Vec<_> = (0..5).map(|_| rect()).collect();
            let mut g = LayoutGrid::new();
            g.columns = vec![Track::new(Unit::Star, 1.0), Track::new(Unit::Star, 1.0)];
            for r in &rs {
                g.children.push(boxed(10.0, 10.0, r.clone(), false));
            }
            perform_layout(&mut g, (0.0, 0.0), (100.0, inf), 1.0);
            let cells: Vec<(f64, f64)> =
                rs.iter().map(|r| (r.borrow().0, r.borrow().1)).collect();
            assert_eq!(
                cells,
                vec![(0.0, 0.0), (50.0, 0.0), (0.0, 10.0), (50.0, 10.0), (0.0, 20.0)]
            );
        }

        // 5. the label reflow grows its auto row
        {
            let tr = rect();
            let mut g = LayoutGrid::new();
            g.columns = vec![Track::new(Unit::Star, 1.0)];
            g.rows = vec![Track::auto()];
            let mut t = boxed(200.0, 20.0, tr.clone(), true);
            t.row = 0; t.col = 0; t.is_label = true;
            t.halign = Align::Near; t.valign = Align::Near;
            g.children = vec![t];
            let (_w, h) = perform_layout(&mut g, (0.0, 0.0), (100.0, inf), 1.0);
            assert_eq!(tr.borrow().3, 40.0);
            assert_eq!(h, 40.0);
        }

        // 6. scale multiplies heights, margins, padding
        {
            let br2 = rect();
            let mut g = LayoutGrid::new();
            g.columns = vec![Track::new(Unit::Star, 1.0)];
            g.rows = vec![Track::new(Unit::Absolute, 10.0)];
            g.padding = (2.0, 3.0, 2.0, 3.0);
            let mut b = boxed(50.0, 999.0, br2.clone(), false);
            b.row = 0; b.col = 0; b.margin = (1.0, 1.0, 1.0, 1.0);
            g.children = vec![b];
            perform_layout(&mut g, (0.0, 0.0), (104.0, 100.0), 2.0);
            let r = *br2.borrow();
            assert_eq!((r.0, r.1), (2.0 * 2.0 + 2.0, 3.0 * 2.0 + 2.0));
            assert_eq!(r.3, 10.0 * 2.0 - 2.0 * 2.0);
            assert_eq!(r.2, 104.0 - 4.0 * 2.0 - 2.0 * 2.0);
        }

        // 7. minimum stretches the last auto
        {
            let (ar2, br3) = (rect(), rect());
            let mut g = LayoutGrid::new();
            g.columns = vec![Track::auto(), Track::auto()];
            g.rows = vec![Track::auto()];
            let mut a = boxed(30.0, 10.0, ar2.clone(), false);
            a.row = 0; a.col = 0; a.halign = Align::Near; a.valign = Align::Near;
            let mut b = boxed(30.0, 10.0, br3.clone(), false);
            b.row = 0; b.col = 1; b.halign = Align::Near; b.valign = Align::Near;
            g.children = vec![a, b];
            let (w, _h) = perform_layout(&mut g, (200.0, 0.0), (300.0, 50.0), 1.0);
            assert_eq!(w, 200.0);
            assert_eq!(br3.borrow().0, 30.0);
        }
    }
}
