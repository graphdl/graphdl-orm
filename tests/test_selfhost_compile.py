"""Self-host, gate two: the CLASSIFIER AUTHORITY is the ingested rules, and dispatch is
the ingested Classification-has-Translator table. compile_model_selfhost runs each
statement as tokenize (Stage-1, the bootstrap kernel) → classify via run_rules →
dispatch to translators named IN M → the translator extracts fields with the Stage-1
productions and asserts. Generic classifications (Fact Type Reading, Instance Fact)
yield to specific ones, mirroring the file's own arbitration-rule values. The gate: the
self-hosted path produces the same fact cells as the seed compiler on a battery covering
pyarest's surface; a kind the rules do not classify (Reference Scheme) is reported
unclassified even though the seed parses it — the rules, not the regex order, decide."""
import pyarest.prims  # noqa: F401
from pyarest.lam import from_lam
from pyarest import forml

MODEL = """Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Person is an entity type.
Party is an entity type.
Passport is an entity type.
Rating is a value type.
The possible values of Rating are 1, 2, 3, 4, 5.
Customer places Order.
Order has Rating.
Order is paid.
Order is not paid.
Each Order has at most one Rating.
Each Order is placed by some Customer.
For each Passport, exactly one Person holds that Passport.
Person holds Passport.
Person is a parent of Person.
Person is a parent of Person is acyclic.
Person is a subtype of Party.
Person1 is an ancestor of Person2 if Person1 is a parent of Person2.
Customer 'c1' places Order 'o1'.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
Transition 'place' is guarded by Fact Type 'Order is paid'.
Transition 'place' emits 'place-receipt'.
Status 'Placed' emits 'awaiting-shipment'.
Order becomes final at depth 6.
"""


def _fact_cells(D):
    """Every cell whose contents are flat fact rows (definitions excluded by shape)."""
    out = {}
    for c in from_lam(D):
        if not (isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"):
            continue
        rows = c[2]
        if isinstance(rows, tuple) and all(
                isinstance(r, tuple) and all(not isinstance(x, tuple) for x in r)
                for r in rows):
            out.setdefault(c[1], set()).update(rows)
    return out


def test_selfhost_compile_matches_the_seed_compiler():
    Dm, _rep1 = forml.compile_model(MODEL)
    Ds, rep2 = forml.compile_model_selfhost(MODEL)
    assert rep2["unclassified"] == []
    assert _fact_cells(Ds) == _fact_cells(Dm)


def test_the_rules_not_the_regex_order_are_the_classifier():
    # the seed parses 'Data Type: …' (its own recognizer); the grammar rules classify
    # only the 'the data type of' surface, which this statement lacks, and it holds no
    # known noun and no rule literal — so the self-hosted path reports it unclassified
    # while the seed accepts it: the rules, not the regex order, are the classifier
    _D, rep = forml.compile_model_selfhost("Person is an entity type.\n"
                                           "Data Type: Salary is Money.\n")
    assert any(s.startswith("Data Type:") for s in rep["unclassified"])
