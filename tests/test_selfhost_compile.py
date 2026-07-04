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


def test_prose_classifies_as_prose_through_the_rules():
    # the flip's last item: the prose posture as GRAMMAR RULES — a paragraph
    # with structural punctuation outside literals classifies Prose (specific
    # beats the generic Fact Type Reading) and reports as prose, never a fact
    # type and never a bare unclassified
    from pyarest import system as S
    model = ("Task is an entity type.\n"
             "Once the engine surfaces facts (issue 821), the Task readiness "
             "derivation fires, materializing what is next.\n")
    D, rep = forml.compile_model_selfhost(model)
    assert any("Once the engine" in s for s in rep.get("prose", []))
    assert not any("Once the engine" in s for s in rep.get("unclassified", []))
    fts = [f[0] for f in S._pop_rows(D, "factType")]
    assert not [ft for ft in fts if "Once" in ft or "readiness" in ft]


def test_selfhost_compiles_atop_a_preloaded_base():
    # flip item 1: the context seam — selfhost apps resolve base-declared
    # types exactly like the seed path (compile_model's context_from)
    from pyarest import meta, system as S
    base_text = ("Status is a value type.\nResource is an entity type.\n"
                 "Resource is currently in Status.\n")
    D0, _ = forml.compile_model(base_text)
    app_text = ("Task is an entity type.\n"
                "Resource 'r1' is currently in Status 'open'.\n")
    D, rep = forml.compile_model_selfhost(app_text, D=D0, context_from=D0)
    assert rep["unclassified"] == []
    rows = {tuple(r) for r in S._pop_rows(D, "Resource_is_currently_in_Status")}
    assert ("r1", "open") in rows


def test_recognizer_tokens_never_fire_inside_literals():
    # task 916's shape: an instance fact whose LITERAL contains a recognizer
    # token (a negation phrase) must classify as an instance fact and land —
    # not dispatch to the negation translator and drop silently
    model = ("Task is an entity type.\nTask Description is a value type.\n"
             "Task has Task Description.\n"
             "Task '916' has Task Description 'matching is not order-sensitive "
             "for unique-role tuples'.\n")
    from pyarest import system as S
    D, rep = forml.compile_model_selfhost(model)
    assert rep["unclassified"] == []
    rows = {tuple(r) for r in S._pop_rows(D, "Task_has_Task_Description")}
    assert ("916", "matching is not order-sensitive for unique-role tuples") in rows


def test_class_rule_twins_equal_the_canonical_path():
    # the FAST twin is a registration; the canonical object is the meaning —
    # the whole selfhost path must answer identically with the registry cleared
    # (generic evaluation) and populated (twins)
    from pyarest import system as S
    forml.grammar_D()                                         # ensure twins built
    saved = dict(S.rule_twins)
    assert saved, "the grammar ingest/thaw must register class-rule twins"
    try:
        S.rule_twins.clear()
        D1, r1 = forml.compile_model_selfhost(MODEL)
        S.rule_twins.update(saved)
        D2, r2 = forml.compile_model_selfhost(MODEL)
    finally:
        S.rule_twins.update(saved)
    assert r1["unclassified"] == r2["unclassified"]
    assert _fact_cells(D1) == _fact_cells(D2)


def test_the_rules_not_the_regex_order_are_the_classifier():
    # the seed's fallback recognizer accepts any sentence; the grammar RULES
    # classify only what carries a recognizer literal, a Role Reference (a
    # Title-case run — the old Stage-1's implicit-noun semantics, which 'Data
    # Type: Salary is Money.' now satisfies and classifies, faithfully), or a
    # Literal Role. An all-lowercase sentence carries none — the self-hosted
    # path reports IT unclassified while the seed accepts it: the rules, not
    # the regex order, are the classifier.
    _D, rep = forml.compile_model_selfhost(
        "Person is an entity type.\n"
        "Data Type: Salary is Money.\n"
        "this sentence resolves to nothing at all.\n")
    assert not any(s.startswith("Data Type:") for s in rep["unclassified"])
    assert any(s.startswith("this sentence") for s in rep["unclassified"])
