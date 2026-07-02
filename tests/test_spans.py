"""Phase 3 step 4 (first half): constraint role bindings are recorded as spans facts in M
(Halpin §13.7's 'Constraint spans Role'), and the positions are computed from the parsed
reading rather than assumed. The uniqueness position is the subject's position in the
fact type's role order; frequency records each named role's position."""
import pyarest.prims  # noqa: F401
from pyarest.lam import from_lam
from pyarest import forml


def _cell(D, name):
    for c in from_lam(D):
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_uniqueness_and_mandatory_record_their_spans():
    D, _ = forml.compile_model("Person is an entity type.\nCountry is an entity type.\n"
                               "Each Person was born in exactly one Country.")
    spans = _cell(D, "spans")
    assert ("Person_was_born_in_Country_uc", 1) in spans
    assert ("Person_was_born_in_Country_mand", 1) in spans


def test_frequency_records_the_named_roles_position():
    D, _ = forml.compile_model("Expert is an entity type.\nPanel is an entity type.\n"
                               "Expert is on Panel.\n"
                               "In each population of Expert is on Panel, "
                               "each Panel combination occurs at least 2 times.")
    assert ("Expert_is_on_Panel_freq", 2) in _cell(D, "spans")   # Panel is role 2


def test_spanning_uc_records_both_positions():
    D, _ = forml.compile_model("Order is an entity type.\nProduct is an entity type.\n"
                               "Order includes Product.\n"
                               "In each population of Order includes Product, "
                               "each Order, Product combination occurs at most once.")
    spans = _cell(D, "spans")
    assert ("Order_includes_Product_uc", 1) in spans and ("Order_includes_Product_uc", 2) in spans
