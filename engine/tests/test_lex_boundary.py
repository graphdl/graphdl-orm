"""The tokenizer boundary (the keystone's transducer set, spec D5's slot: value
ops on names are registered boundary ops, like cellkey). Three primitives carry
text into the object world; everything above them — the mixfix scan, the type
spans, Stage-1's vocabulary matcher — is sequence algebra, canonical territory.

  lex:     text → ⟨⟨raw, nopunct, base, subscript, lower, qtext, title, tpl,
           quoted, qidx⟩…⟩ per whitespace word. nopunct strips '.;:,' at both
           ends (the _atomic_run_guard strip); base further strips trailing
           digits and subscript keeps them (Halpin's Task1 twins); lower is the
           case-fold (Stage-1 matches case-insensitively); qtext is the word's
           text INSIDE its quoted span, quotes excluded (the _QUOTED span is
           character-level, so a trailing period outside the closing quote
           stays out of the literal); title is T iff base opens uppercase; tpl
           is the word's TEMPLATE form under NORMA hyphen binding (#24 — a
           one-sided touching hyphen is the bind marker, consumed: 'adj-'/
           '-adj' -> the word; the doubled hyphen escapes to one literal
           hyphen: 'FORE--' -> 'FORE-'; a hyphen touching both sides is just a
           word); quoted/qidx mark span membership, spans numbered from 1.
  implode: ⟨sep, ⟨w…⟩⟩ → one atom (templates are STRINGS in factType rows).
  slug:    text → id atom (the [^0-9A-Za-z]+ → '_' collapse, ends stripped —
           id MINTING is a boundary act; names are data)."""
import pyarest.prims  # noqa: F401
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest.reduce import apply


def _lex(text):
    return from_lam(apply(A("lex"), to_lam(text)))


def test_lex_plain_words_carry_their_case():
    got = _lex("Order was placed by Customer")
    assert [r[0] for r in got] == ["Order", "was", "placed", "by", "Customer"]
    assert [r[6] for r in got] == ["T", "F", "F", "F", "T"]   # title?
    r = got[0]
    assert r == ("Order", "Order", "Order", "", "order", "", "T", "Order", "F", 0)


def test_lex_strips_punctuation_and_splits_subscripts():
    got = _lex("Each Task1 occurs.")
    each, task1, occurs = got
    assert each[4] == "each"                                  # lower, for Stage-1
    assert (task1[1], task1[2], task1[3], task1[6]) == ("Task1", "Task", "1", "T")
    assert (occurs[0], occurs[1], occurs[2]) == ("occurs.", "occurs", "occurs")


def test_lex_marks_quoted_spans_character_wise():
    got = _lex("Status 'In Cart' is to Status 'Placed'.")
    by_raw = {r[0]: r for r in got}
    assert by_raw["Status"][8] == "F"
    assert (by_raw["'In"][8], by_raw["'In"][9], by_raw["'In"][5]) == ("T", 1, "In")
    assert (by_raw["Cart'"][9], by_raw["Cart'"][5]) == (1, "Cart")
    # the closing token's period sits OUTSIDE the span: the literal is Placed
    assert (by_raw["'Placed'."][8], by_raw["'Placed'."][9],
            by_raw["'Placed'."][5]) == ("T", 2, "Placed")


def test_lex_hyphen_carries_the_norma_template_form():
    got = _lex("Layer has valence- Coord and Type -adj and FORE-- WORD and from-Status")
    by_raw = {r[0]: r for r in got}
    assert by_raw["valence-"][7] == "valence"                 # leading bind: marker consumed
    assert by_raw["-adj"][7] == "adj"                         # trailing bind: marker consumed
    assert by_raw["FORE--"][7] == "FORE-"                     # -- escape: one literal hyphen
    assert by_raw["from-Status"][7] == "from-Status"          # touching: just a word (retired bind)
    assert by_raw["Layer"][7] == "Layer"                      # plain words pass through


def test_implode_joins_template_tokens():
    assert from_lam(apply(A("implode"), to_lam((" ", ("{0}", "was", "placed", "by", "{1}"))))) \
        == "{0} was placed by {1}"
    assert from_lam(apply(A("implode"), to_lam(("_", ("a", "b", "c"))))) == "a_b_c"


def test_slug_mints_the_id():
    assert from_lam(apply(A("slug"), to_lam("Order was placed by Customer"))) \
        == "Order_was_placed_by_Customer"
    assert from_lam(apply(A("slug"), to_lam("place-receipt"))) == "place_receipt"
    assert from_lam(apply(A("slug"), to_lam(" Person owns Task "))) == "Person_owns_Task"


def test_lex_agrees_with_the_host_split_over_the_corpus():
    """Twin sanity over real model text: raws are exactly text.split(), and the
    per-word attributes match the host expressions they replace."""
    from pyarest import forml
    MODEL = """Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Customer places Order.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Each Order is placed by at most one Customer2.
Layer has valence- Coord.
Transition is from- Status.
Layer has Coord -local and FORE-- WORD and from-Status.
"""
    for line in MODEL.strip().split("\n"):
        got = _lex(line)
        toks = line.split()
        assert [r[0] for r in got] == toks
        for r, tok in zip(got, toks):
            base = tok.strip(".;:,").rstrip("0123456789")
            assert r[1] == tok.strip(".;:,")
            assert r[2] == base
            assert r[3] == tok.strip(".;:,")[len(base):]
            assert r[4] == tok.lower()
            assert r[6] == ("T" if base and base[0].isupper() else "F")
            assert r[7] == forml._hyphen_tpl(tok)             # field 8 twins the host collapse
