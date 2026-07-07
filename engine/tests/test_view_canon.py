"""The view projections as CANON (the ui re-authoring's first slice,
design of record 2026-07-08): view(entity) = project(P, entity) —
Theorem 4 extended from actions to whole views, as PURE DEFS answering
abstract element trees as VALUES (never stored facts; the binding
doctrine, AREST.tex §Platform binding). system:view_menu wraps the
proven transitions fold: applied to ⟨status, sm-triples⟩ it answers
⟨"menu", ⟨⟨"button", event, to⟩…⟩⟩ — one button per transition
available FROM the status, in the machine's row order. The element
vocabulary is the Component registry's Role names ('button'), so
select_component resolves each node to a toolkit implementation and a
DEFS-registered render function draws it: a fact renders itself."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, from_lam, to_lam
from pyarest.reduce import apply


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


SM = (("t1", "pending", "start", "in_progress"),
      ("t2", "in_progress", "finish", "completed"),
      ("t3", "in_progress", "block", "blocked"),
      ("t4", "blocked", "unblock", "in_progress"))


def _menu(status):
    sm = to_lam(tuple((f, e, t) for (_i, f, e, t) in SM))
    out = from_lam(apply(A("system:view_menu"), _S(A(status), sm)))
    return out


def test_the_menu_projects_the_available_transitions():
    got = _menu("in_progress")
    assert isinstance(got, tuple) and got[0] == "menu"
    buttons = got[1]
    assert buttons == (("button", "finish", "completed"),
                       ("button", "block", "blocked"))


def test_a_status_with_no_transitions_answers_an_empty_menu():
    got = _menu("completed")
    assert got == ("menu", ())


def test_the_detail_view_projects_the_entitys_pairs():
    # view_detail over verbalize's pairs (⟨reading, row⟩…): one field per
    # fact, the reading IS the caption (§4.4 — richer than MT.D's
    # MakeCaption), the row rides whole for the renderer's wording. The
    # value-type → Component Role refinement (§4.2) lands when those
    # facts ingest; 'field' is the neutral role meanwhile.
    pairs = to_lam((("{0} has {1}", ("t1", "open")),
                    ("{0} keeps {1}", ("t1", "rex"))))
    got = from_lam(apply(A("system:view_detail"), pairs))
    assert got == ("detail",
                   (("field", "{0} has {1}", ("t1", "open")),
                    ("field", "{0} keeps {1}", ("t1", "rex"))))


def test_an_entity_with_no_facts_answers_an_empty_detail():
    got = from_lam(apply(A("system:view_detail"), to_lam(())))
    assert got == ("detail", ())


def test_the_escape_transducer_is_the_only_boundary_piece():
    # the doctrine correction (Samuel, 2026-07-08): meaning in canon,
    # boundary for TRANSDUCTION only. escape_html is the one byte-level
    # piece the render needs — & < > " to their entities; ints stringify;
    # sequences bottom (a value op, the lex family).
    assert from_lam(apply(A("escape_html"), A('a<b>&"c'))) == \
        "a&lt;b&gt;&amp;&quot;c"
    assert from_lam(apply(A("escape_html"), A(7))) == "7"
    assert from_lam(apply(A("escape_html"), _S(A("x")))) == "⊥"


def test_the_reference_html_render_is_a_registered_def():
    # "Binding a user interface is then registering a render function, so
    # a fact renders itself" (AREST.tex §Platform binding, verbatim). The
    # engine ships ONE reference renderer as a D5 boundary op — semantic
    # html per tree kind; toolkit renderers register beside it the same
    # way (the iFactr pattern).
    menu = to_lam(("menu", (("button", "finish", "completed"),
                            ("button", "block", "blocked"))))
    html = from_lam(apply(A("render:html"), menu))
    assert html == ('<nav class="menu">'
                    '<button name="finish" value="completed">finish</button>'
                    '<button name="block" value="blocked">block</button>'
                    '</nav>')
    detail = to_lam(("detail", (("field", "{0} has {1}", ("t1", "open")),)))
    html = from_lam(apply(A("render:html"), detail))
    assert html == ('<dl class="detail">'
                    '<dt>{0} has {1}</dt><dd>t1 open</dd>'
                    '</dl>')
    lst = to_lam(("list", (("item", "t1", "fix the door"),)))
    html = from_lam(apply(A("render:html"), lst))
    assert html == ('<ul class="list">'
                    '<li data-id="t1">fix the door</li>'
                    '</ul>')
    assert from_lam(apply(A("render:html"), A("nonsense"))) == "⊥"


def test_the_canon_render_twins_the_host_override():
    # the doctrine conformance (Samuel's correction, 2026-07-08):
    # system:render_html is the DEFINITION OF RECORD; the host render
    # (render:html) is its certified-equal performance override. The
    # twin holds byte-equal on all three tree kinds + escaping.
    trees = [
        ("menu", (("button", "finish", "completed"),
                  ("button", "block", "blocked"))),
        ("menu", ()),
        ("detail", (("field", "{0} has {1}", ("t1", "open")),
                    ("field", "{0} <keeps> {1}", ("t1", 'r"x')))),
        ("list", (("item", "t1", "fix the door"),
                  ("item", "t2", "a<b"))),
        ("list", ()),
    ]
    for t in trees:
        canon = from_lam(apply(A("system:render_html"), to_lam(t)))
        host = from_lam(apply(A("render:html"), to_lam(t)))
        assert canon == host, (t, canon, host)
        assert isinstance(canon, str) and canon.startswith("<")


def test_the_list_view_projects_id_caption_rows():
    # view_list over ⟨id, caption⟩ rows (the host feeds the ref scheme's
    # identifier + the summarising fact per §3.1): one item per instance.
    rows = to_lam((("t1", "fix the door"), ("t2", "paint it")))
    got = from_lam(apply(A("system:view_list"), rows))
    assert got == ("list",
                   (("item", "t1", "fix the door"),
                    ("item", "t2", "paint it")))
    assert from_lam(apply(A("system:view_list"), to_lam(()))) == ("list", ())
