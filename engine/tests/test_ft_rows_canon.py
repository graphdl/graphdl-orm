"""The fact-type declaration translator as a canonical object — the L1
translator arc's LAST family. system:ft_rows — ⟨template, roles, kind⟩ →
the M-rows a declaration asserts (the sm_rows shape): the factType row
⟨ft, template⟩, one role row ⟨ft.i, ft, i, player⟩ per role in order, and
the derivation link ⟨ft, kind⟩ when a storage marker rode the reading.
The scan half is already canon (reading_parse/ftid); _fact_type's plan
assembly is the behavioral spec (expectations probed from it verbatim);
_h_fact stays the boundary (instance branch, subtype lift, the strip)."""
import pyarest.prims  # noqa: F401
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest.reduce import apply


def _rows(template, roles, kind):
    got = from_lam(apply(A("system:ft_rows"),
                         to_lam((template, tuple(roles), kind))))
    assert isinstance(got, tuple), got          # ⊥ = the def is missing
    return got


def test_a_binary_declaration_asserts_facttype_and_ordered_roles():
    assert _rows("{0} was placed by {1}", ("Order", "Customer"), "") == (
        ("factType", ("Order_was_placed_by_Customer",
                      "{0} was placed by {1}")),
        ("role", ("Order_was_placed_by_Customer.1",
                  "Order_was_placed_by_Customer", 1, "Order")),
        ("role", ("Order_was_placed_by_Customer.2",
                  "Order_was_placed_by_Customer", 2, "Customer")),
    )


def test_a_unary_declaration_asserts_one_role():
    assert _rows("{0} smokes", ("Person",), "") == (
        ("factType", ("Person_smokes", "{0} smokes")),
        ("role", ("Person_smokes.1", "Person_smokes", 1, "Person")),
    )


def test_a_derivation_marker_appends_the_link():
    got = _rows("{0} smokes", ("Person",), "fully-derived")
    assert got[-1] == ("derivation", ("Person_smokes", "fully-derived"))
    assert got[:-1] == _rows("{0} smokes", ("Person",), "")


def test_the_canonical_plan_twins_the_python_over_the_corpus():
    """Every fact-type DECLARATION in shared/base asserts the same M-rows
    through system:ft_rows as through _fact_type (the scan supplies
    template+roles to both; the plan is what's compared)."""
    import os
    from pyarest import forml
    root = os.path.join(os.path.dirname(os.path.dirname(
        os.path.abspath(__file__))), "shared", "base")
    text = "\n\n".join(open(os.path.join(root, f), encoding="utf-8").read()
                       for f in sorted(os.listdir(root)) if f.endswith(".md"))
    stmts = forml.statements(text)
    known = forml._known(stmts)
    checked = 0
    for s in stmts:
        kind, g, _m = forml.analyze(s)
        if kind != "fact_type_reading" or "'" in g[0] \
                or forml._prose_suspect(g[0], known):
            continue
        reading = forml._strip_derivation(g[0])[1]
        _ft, want = forml._fact_type(reading, known)
        if not want:
            continue                       # resolved to a declared ft, no plan
        template, roles = forml._reading(reading, known)
        got = _rows(template, tuple(roles), "")
        assert got == tuple((c, tuple(r)) for (c, r) in want), (reading, got)
        checked += 1
    assert checked >= 50                                # the corpus is real
