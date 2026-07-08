"""The standard-cell compositions (iFactr's ContentCell /
HeaderedControlCell recipes as shared code): the SAME grid every
platform lays out — text star column, right-aligned value, subtext
spanning, the image bleeding into the padding."""
from pyarest.uicells import (
    CELL_HEIGHT, content_cell, headered_control_cell,
)
from pyarest.uilayout import INF, perform_layout


def _box(w, h):
    rect = []

    def measure(cw, ch):
        return (min(w, cw), min(h, ch))

    def place(x, y, pw, ph):
        rect[:] = [x, y, pw, ph]
    return (measure, place), rect


def test_content_cell_composition():
    (tm, tp), text = _box(120, 18)
    (vm, vp), value = _box(60, 16)
    (sm, sp), sub = _box(200, 14)
    (im, ip), img = _box(32, 32)
    g = content_cell((tm, tp), value=(vm, vp), subtext=(sm, sp),
                     image=(im, ip))
    w, h = perform_layout(g, (320, CELL_HEIGHT), (320, INF))
    assert w == 320
    # the image bleeds into the left padding (origin 16 - 16 = 0)
    assert img[0] == 0
    # text starts at the image's right edge + the large spacing (the
    # bleed cancels the padding: 0 + 32 + 10); value right-aligned
    assert text[0] == 42
    assert value[0] + value[2] == 320 - 16
    # subtext on the second row, under the text
    assert sub[1] > text[1]


def test_content_cell_without_image_or_value():
    (tm, tp), text = _box(120, 18)
    g = content_cell((tm, tp))
    w, h = perform_layout(g, (320, CELL_HEIGHT), (320, INF))
    assert text[0] == 16                     # the left margin
    assert h == CELL_HEIGHT                  # floored by the minimum


def test_headered_control_cell_rows():
    (hm, hp), header = _box(100, 18)
    (c1m, c1p), c1 = _box(280, 24)
    (c2m, c2p), c2 = _box(280, 24)
    g = headered_control_cell((hm, hp), [(c1m, c1p), (c2m, c2p)])
    perform_layout(g, (320, 0), (320, INF))
    assert header[1] < c1[1] < c2[1]         # header, then each control
    assert c1[0] == 16 and c1[2] == 320 - 32  # stretched to the star
