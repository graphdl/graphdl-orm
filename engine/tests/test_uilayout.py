"""The abstract layout invariants (iFactr GridExtensions.PerformLayout,
ported 2026-07-08 on Samuel's correction: these ARE how abstract
layouts work). Deterministic fake controls pin the engine's mechanics:
track sizing, spans, margins, alignment, auto-placement flow, the
label reflow, star weights, and the scale treatment."""
from pyarest.uilayout import (
    ABSOLUTE, AUTO, CENTER, FAR, INF, NEAR, STAR, STRETCH,
    Element, Grid, Track, perform_layout,
)


class Box:
    """A fake control with a fixed desired size and a placement log."""

    def __init__(self, w, h, **kw):
        self.w, self.h = float(w), float(h)
        self.rect = None
        self.el = Element(self.measure, self.place, **kw)

    def measure(self, cw, ch):
        return (min(self.w, cw), min(self.h, ch))

    def place(self, x, y, w, h):
        self.rect = (x, y, w, h)


class WrapText(Box):
    """Area-preserving text: narrower means taller (the label rule)."""

    def measure(self, cw, ch):
        w = min(self.w, cw)
        h = min(self.h * (self.w / max(w, 1.0)), ch)
        return (w, h)


def test_label_column_and_star_value_column():
    # the detail row: auto label column takes its widest label; the
    # star value column takes the rest
    g = Grid(columns=[Track(AUTO), Track(STAR)],
             rows=[Track(AUTO), Track(AUTO)])
    k1 = Box(40, 20, row=0, col=0, halign=NEAR, valign=NEAR)
    v1 = Box(500, 20, row=0, col=1)
    k2 = Box(60, 20, row=1, col=0, halign=NEAR, valign=NEAR)
    v2 = Box(30, 20, row=1, col=1, halign=NEAR, valign=NEAR)
    for b in (k1, v1, k2, v2):
        g.add(b.el)
    w, h = perform_layout(g, (0, 0), (300, INF))
    assert (w, h) == (300, 40)
    assert k1.rect == (0, 0, 40, 20)
    assert k2.rect == (0, 20, 60, 20)
    # the star column starts after the WIDEST label and stretches
    assert v1.rect == (60, 0, 240, 20)
    assert v2.rect[0] == 60 and v2.rect[2] == 30


def test_absolute_star_weights_and_padding():
    g = Grid(columns=[Track(ABSOLUTE, 50), Track(STAR, 1), Track(STAR, 3)],
             rows=[Track(ABSOLUTE, 30)], padding=(10, 5, 10, 5))
    a = Box(999, 999, row=0, col=0)
    b = Box(999, 999, row=0, col=1)
    c = Box(999, 999, row=0, col=2)
    for x in (a, b, c):
        g.add(x.el)
    perform_layout(g, (0, 0), (270, 40))
    # available = 270 - 20 padding - 50 absolute = 200; star unit 50
    assert a.rect == (10, 5, 50, 30)
    assert b.rect == (60, 5, 50, 30)
    assert c.rect == (110, 5, 150, 30)


def test_margins_and_alignments():
    g = Grid(columns=[Track(ABSOLUTE, 100)], rows=[Track(ABSOLUTE, 100)])
    near = Box(20, 10, row=0, col=0, halign=NEAR, valign=NEAR,
               margin=(5, 6, 0, 0))
    far = Box(20, 10, row=0, col=0, halign=FAR, valign=FAR,
              margin=(0, 0, 5, 6))
    ctr = Box(20, 10, row=0, col=0, halign=CENTER, valign=CENTER)
    fill = Box(20, 10, row=0, col=0)          # stretch default
    for x in (near, far, ctr, fill):
        g.add(x.el)
    perform_layout(g, (0, 0), (100, 100))
    assert near.rect == (5, 6, 20, 10)
    assert far.rect == (75, 84, 20, 10)
    assert ctr.rect == (40, 45, 20, 10)
    assert fill.rect == (0, 0, 100, 100)


def test_auto_placement_flows_rows():
    # AutoLayoutIndex children flow: each lands in the next free cell
    # of a 2-column grid, growing rows as needed
    g = Grid(columns=[Track(STAR), Track(STAR)])
    boxes = [Box(10, 10) for _ in range(5)]
    for b in boxes:
        g.add(b.el)
    perform_layout(g, (0, 0), (100, INF))
    cells = [(b.rect[0], b.rect[1]) for b in boxes]
    assert cells == [(0.0, 0.0), (50.0, 0.0),
                     (0.0, 10.0), (50.0, 10.0),
                     (0.0, 20.0)]


def test_the_label_reflow_grows_its_auto_row():
    # a wrapping label constrained to half its width doubles in height;
    # the second measure against infinite height grows the auto row
    g = Grid(columns=[Track(STAR)], rows=[Track(AUTO)])
    t = WrapText(200, 20, row=0, col=0, is_label=True, valign=NEAR,
                 halign=NEAR)
    g.add(t.el)
    w, h = perform_layout(g, (0, 0), (100, INF))
    assert t.rect[3] == 40.0          # reflowed height
    assert h == 40.0                  # the row grew to fit


def test_scale_multiplies_heights_margins_padding():
    g = Grid(columns=[Track(STAR)], rows=[Track(ABSOLUTE, 10)],
             padding=(2, 3, 2, 3))
    b = Box(50, 999, row=0, col=0, margin=(1, 1, 1, 1))
    g.add(b.el)
    perform_layout(g, (0, 0), (104, 100), scale=2.0)
    x, y, w, h = b.rect
    assert (x, y) == (2 * 2 + 1 * 2, 3 * 2 + 1 * 2)
    assert h == 10 * 2 - 2 * 2        # absolute row scaled, margins off
    assert w == 104 - 4 * 2 - 2 * 2   # width padding+margins scaled too


def test_minimum_stretches_the_last_auto():
    g = Grid(columns=[Track(AUTO), Track(AUTO)], rows=[Track(AUTO)])
    a = Box(30, 10, row=0, col=0, halign=NEAR, valign=NEAR)
    b = Box(30, 10, row=0, col=1, halign=NEAR, valign=NEAR)
    g.add(a.el)
    g.add(b.el)
    w, _h = perform_layout(g, (200, 0), (300, 50))
    assert w == 200                   # floored by the minimum
    # the LAST non-zero auto column absorbed the slack
    assert b.rect[0] == 30
