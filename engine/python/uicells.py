"""The standard-cell compositions — iFactr's ContentCell and
HeaderedControlCell recipes as shared code (the Instructor layer: the
platform-agnostic grid compositions the layout engine consumes; the
platform Instructors only refine them — Compact bumps MinHeight for
subtext, Android rebuilds beside-layouts).

A composition answers a uilayout.Grid; the caller supplies element
FACTORIES (label/value/image constructors returning uilayout.Element
kwargs-ready measure/place pairs), so the same recipe drives tk, WPF,
or a framebuffer. Metrics are iFactr's (Thickness: left/right margin
16, top/bottom 10, small spacing 7, large 10; StandardCellHeight 48).
"""
from .uilayout import (
    AUTO, CENTER, FAR, NEAR, STAR, Element, Grid, Track,
)

LEFT_MARGIN = 16
RIGHT_MARGIN = 16
TOP_MARGIN = 10
BOTTOM_MARGIN = 10
SMALL_SPACING = 7
LARGE_SPACING = 10
CELL_HEIGHT = 48


def content_cell(text, value=None, subtext=None, image=None):
    """ContentCell: ⟨auto image | star text | auto value⟩ over two auto
    rows — TextLabel row 0 col 1, ValueLabel right-aligned row 0 col 2,
    SubtextLabel row 1 spanning cols 1-2, the Image spanning both rows
    and bleeding into the cell padding (the negative margins). Elements
    arrive as (measure, place) pairs; absent parts are omitted exactly
    as Layout() removes the pathless Image."""
    g = Grid(rows=[Track(AUTO), Track(AUTO)],
             columns=[Track(AUTO), Track(STAR), Track(AUTO)],
             padding=(LEFT_MARGIN, TOP_MARGIN, RIGHT_MARGIN,
                      BOTTOM_MARGIN))
    if image is not None:
        m, p = image
        g.add(Element(m, p, row=0, col=0, row_span=2,
                      margin=(-LEFT_MARGIN, -TOP_MARGIN, LARGE_SPACING,
                              -BOTTOM_MARGIN),
                      halign=NEAR, valign=CENTER))
    m, p = text
    g.add(Element(m, p, row=0, col=1, halign=NEAR, valign=CENTER,
                  is_label=True))
    if value is not None:
        m, p = value
        g.add(Element(m, p, row=0, col=2, halign=FAR, valign=CENTER,
                      margin=(SMALL_SPACING, 0, 0, 0), is_label=True))
    if subtext is not None:
        m, p = subtext
        g.add(Element(m, p, row=1, col=1, col_span=2, halign=NEAR,
                      valign=CENTER, is_label=True))
    return g


def headered_control_cell(header, controls):
    """HeaderedControlCell: one star column; the header on its own auto
    row, then one auto row per control."""
    g = Grid(columns=[Track(STAR)],
             rows=[Track(AUTO)] + [Track(AUTO) for _ in controls],
             padding=(LEFT_MARGIN, TOP_MARGIN, RIGHT_MARGIN,
                      BOTTOM_MARGIN))
    m, p = header
    g.add(Element(m, p, row=0, col=0, halign=NEAR, valign=CENTER,
                  is_label=True))
    for i, (m, p) in enumerate(controls, start=1):
        g.add(Element(m, p, row=i, col=0, valign=NEAR))
    return g
