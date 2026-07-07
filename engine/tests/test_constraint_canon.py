"""The constraint TRANSLATOR as a canonical object (the family after
clause_ft). system:cs_rows — ⟨kind, subject, clause ids, raw texts, m⟩ →
the constraint A-row plus the attachment plan as ⟨attach-cell,
builder-name⟩ rows: WHICH constraint fact a statement asserts and WHERE
each scoped object attaches, as data. The C.* builders are ALREADY
canonical (constraints:scoped_* — engine.py's no-pops defaults); the four
handlers (compiler.py 874-906) stay thin callers that mint cids at the
boundary ([:40] truncation is boundary policy, the sm_rows doctrine — no
take prim joins the kernel for cosmetics) and fold attachment rows
through the named builders. Expected literals below trace the handlers
verbatim; canon emits FULL slug cids."""
import pyarest.prims  # noqa: F401
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest.reduce import apply


def _rows(kind, subject, clauses, raws, m):
    return from_lam(apply(A("system:cs_rows"),
                          to_lam((kind, subject, tuple(clauses),
                                  tuple(raws), m))))


def test_at_most_one_emits_exclusion_and_per_clause_attachments():
    got = _rows("at most", "Ticket", ("Ticket_is_open", "Ticket_is_closed"),
                ("Ticket is open", "Ticket is closed"), "M1")
    assert got == (
        ("constraint", "Ticket_excl", "exclusion", "Ticket",
         ("Ticket_is_open", "Ticket_is_closed"), "M1"),
        ("attach", "Ticket_excl", "exclusion"),
        ("attach", "Ticket_excl@Ticket_is_open", "scoped_exclusion"),
        ("attach", "Ticket_excl@Ticket_is_closed", "scoped_exclusion"),
    )


def test_exactly_one_emits_exclusive_or():
    got = _rows("exactly", "Ticket", ("Ticket_is_open", "Ticket_is_closed"),
                ("Ticket is open", "Ticket is closed"), "M2")
    assert got == (
        ("constraint", "Ticket_xor", "exclusive_or", "Ticket",
         ("Ticket_is_open", "Ticket_is_closed"), "M2"),
        ("attach", "Ticket_xor", "exclusive_or"),
        ("attach", "Ticket_xor@Ticket_is_open", "scoped_exclusive_or"),
        ("attach", "Ticket_xor@Ticket_is_closed", "scoped_exclusive_or"),
    )


def test_disjunctive_emits_inclusive_or():
    got = _rows("disjunctive_mandatory", "Person",
                ("Person_has_Email", "Person_has_Phone"),
                ("Person has Email", "Person has Phone"), "M3")
    assert got == (
        ("constraint", "ior_Person", "disjunctive_mandatory", "Person",
         ("Person_has_Email", "Person_has_Phone"), "M3"),
        ("attach", "ior_Person", "inclusive_or"),
        ("attach", "ior_Person@Person_has_Email", "scoped_inclusive_or"),
        ("attach", "ior_Person@Person_has_Phone", "scoped_inclusive_or"),
    )


def test_subset_slugs_the_antecedent_text_and_attaches_once():
    got = _rows("subset", "",
                ("Person_smokes", "Person_is_adult"),
                ("Person smokes", "Person is adult"), "M4")
    assert got == (
        ("constraint", "subset_Person_smokes", "subset",
         "Person_smokes", "Person_is_adult", "M4"),
        ("attach", "subset_Person_smokes", "scoped_subset"),
    )


def test_equality_attaches_both_sides():
    got = _rows("equality", "",
                ("Person_was_born", "Person_has_Birthdate"),
                ("Person was born", "Person has Birthdate"), "M5")
    assert got == (
        ("constraint", "eq_Person_was_born", "equality",
         "Person_was_born", "Person_has_Birthdate", "M5"),
        ("attach", "eq_Person_was_born_a", "scoped_equality_side"),
        ("attach", "eq_Person_was_born_b", "scoped_equality_side"),
    )
