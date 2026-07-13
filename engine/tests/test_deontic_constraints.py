"""The deontic constraint family, target semantics read off the old
message-vetting store: a deontic statement DECLARES its inner proposition's
fact type into the schema (population stays empty) and mints one constraint
row carrying the operator, the fact type span, the quoted values if any, and
the deontic modality tail. Deontic flags, never blocks (Def. Violation)."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system


def _roles(D, ft):
    out = []
    for r in system._pop_rows(D, "role"):
        if len(r) >= 4 and r[1] == ft:
            out.append((r[2], r[3]))
    return sorted(out)


def _deontic_rows(D):
    return [tuple(c) for c in system._pop_rows(D, "constraint")
            if len(c) >= 2 and str(c[1]).startswith("deontic")]


def test_a_forbidden_statement_declares_the_shape_and_the_constraint():
    # the old store's encoding for 'It is forbidden that Message contains
    # Markdown Syntax.': the fact type exists with roles Message and
    # Markdown Syntax and an EMPTY population, and one constraint row rides
    # with operator forbidden and modality deontic
    model = """Message is an entity type.
Markdown Syntax is a value type.

It is forbidden that Message contains Markdown Syntax.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    assert _roles(D, "Message_contains_Markdown_Syntax") == [
        (1, "Message"), (2, "Markdown Syntax")]
    assert system._pop_rows(D, "Message_contains_Markdown_Syntax") == []
    rows = _deontic_rows(D)
    assert len(rows) == 1
    c = rows[0]
    assert c[0] == "It is forbidden that Message contains Markdown Syntax"
    assert c[1] == "deontic_forbidden"
    assert c[2] == "Message_contains_Markdown_Syntax"
    assert c[-1] == "deontic"


def test_the_quantified_obligation_strips_each_from_the_shape():
    # the old store's answer for 'It is obligatory that each Message is
    # natural.': the fact type is Message_is_natural (the quantifier never
    # rides into the schema), while the constraint text keeps the statement
    model = """Message is an entity type.

It is obligatory that each Message is natural.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    assert _roles(D, "Message_is_natural") == [(1, "Message")]
    rows = _deontic_rows(D)
    assert len(rows) == 1
    assert rows[0][0] == "It is obligatory that each Message is natural"
    assert rows[0][2] == "Message_is_natural"


def _mk(tmp_path, model):
    base = tmp_path / "apps"
    (base / "mv" / "readings").mkdir(parents=True)
    (base / "mv" / "readings" / "app.md").write_text(model, encoding="utf-8")
    from pyarest import apps as _apps
    return _apps.Registry(str(base), cache_dir=str(tmp_path / "fz"))


def test_validate_flags_a_forbidden_population_and_never_blocks(tmp_path):
    # the vetting behavior, population form (the old DF_pop kind): the
    # forbidden fact COMMITS (deontic never blocks, Def. Violation) and
    # validate flags every row of the forbidden population, alethic False
    reg = _mk(tmp_path, "Message is an entity type.\n"
                        "Markdown Syntax is a value type.\n\n"
                        "It is forbidden that Message contains "
                        "Markdown Syntax.\n")
    reg.compile("mv")
    r = reg.apply("mv", "Message_contains_Markdown_Syntax",
                  ("m1", "**bold**"))
    assert r["committed"]
    out = reg.validate("mv")
    hits = [v for v in out["violations"]
            if v["fact_type"] == "Message_contains_Markdown_Syntax"]
    assert len(hits) == 1
    assert ["m1", "**bold**"] in hits[0]["offenders"]
    assert hits[0]["alethic"] is False


def test_validate_flags_only_rows_carrying_a_forbidden_value(tmp_path):
    # the closed-world value form (the old DF_cwa kind): only the row
    # carrying the forbidden value flags; the clean row stays clean
    reg = _mk(tmp_path, "API is an entity type.\n"
                        "Message is an entity type.\n"
                        "Field Name is a value type.\n"
                        "API Product is a subtype of API.\n"
                        "Message names API Product by Field Name.\n\n"
                        "It is forbidden that Message names API Product "
                        "by Field Name 'EndpointSlug'.\n")
    reg.compile("mv")
    ft = "Message_names_API_Product_by_Field_Name"
    assert reg.apply("mv", ft, ("m1", "p1", "EndpointSlug"))["committed"]
    assert reg.apply("mv", ft, ("m2", "p1", "Docs"))["committed"]
    out = reg.validate("mv")
    hits = [v for v in out["violations"] if v["fact_type"] == ft]
    assert len(hits) == 1
    assert hits[0]["offenders"] == [["m1", "p1", "EndpointSlug"]]
    assert hits[0]["alethic"] is False


def test_validate_flags_rows_missing_an_obligated_value(tmp_path):
    # the obligatory value form (the old DO_pop kind): a row that lacks the
    # obligated value flags; the conforming row stays clean; nothing blocks
    reg = _mk(tmp_path, "API is an entity type.\n"
                        "Message is an entity type.\n"
                        "Field Name is a value type.\n"
                        "API Product is a subtype of API.\n"
                        "Message names API Product by Field Name.\n\n"
                        "It is obligatory that Message names API Product "
                        "by Field Name 'Title'.\n")
    reg.compile("mv")
    ft = "Message_names_API_Product_by_Field_Name"
    assert reg.apply("mv", ft, ("m1", "p1", "Title"))["committed"]
    assert reg.apply("mv", ft, ("m2", "p1", "Docs"))["committed"]
    out = reg.validate("mv")
    hits = [v for v in out["violations"] if v["fact_type"] == ft]
    assert len(hits) == 1
    assert hits[0]["offenders"] == [["m2", "p1", "Docs"]]
    assert hits[0]["alethic"] is False


def test_validate_flags_subjects_missing_a_bare_obligation(tmp_path):
    # the bare obligatory form (the old DO_obl kind) IS a mandatory
    # constraint with deontic modality: every subject instance must play
    # the obligated fact type; the one that does not flags, nothing blocks
    reg = _mk(tmp_path, "Message is an entity type.\n"
                        "Body is a value type.\n"
                        "Pricing Model is a value type.\n"
                        "Message has Body.\n"
                        "Each Message has at most one Body.\n"
                        "Message conforms to Pricing Model.\n\n"
                        "It is obligatory that Message conforms to "
                        "Pricing Model.\n")
    reg.compile("mv")
    assert reg.apply("mv", "Message_has_Body", ("m1", "hello"))["committed"]
    assert reg.apply("mv", "Message_has_Body", ("m2", "world"))["committed"]
    assert reg.apply("mv", "Message_conforms_to_Pricing_Model",
                     ("m1", "standard"))["committed"]
    out = reg.validate("mv")
    hits = [v for v in out["violations"]
            if v["fact_type"] == "Message_conforms_to_Pricing_Model"]
    assert len(hits) == 1
    offenders = {tuple(x) for x in hits[0]["offenders"]}
    assert ("m2",) in offenders and not any("m1" in o for o in offenders)
    assert hits[0]["alethic"] is False


def test_the_quoted_deontic_pair_binds_values_and_mints_no_rows():
    # the Field Name pair: the quoted value rides the constraint row, the
    # population stays empty, and obligatory versus forbidden splits on the
    # operator column
    model = """API is an entity type.
Message is an entity type.
Field Name is a value type.
API Product is a subtype of API.
Message names API Product by Field Name.

It is obligatory that Message names API Product by Field Name 'Title'.
It is forbidden that Message names API Product by Field Name 'EndpointSlug'.
"""
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    assert system._pop_rows(
        D, "Message_names_API_Product_by_Field_Name") == []
    rows = _deontic_rows(D)
    by_op = {c[1]: c for c in rows}
    assert set(by_op) == {"deontic_obligatory", "deontic_forbidden"}
    ob = by_op["deontic_obligatory"]
    fb = by_op["deontic_forbidden"]
    assert ob[2] == fb[2] == "Message_names_API_Product_by_Field_Name"
    assert ("Title",) in ob and ("EndpointSlug",) in fb


def test_compound_deontic_does_not_mint_a_phantom_fact_type():
    # #34: 'It is forbidden that X and that Y and that Z' is a multi-fact-type
    # JOIN exclusion, not one fact type. The fact_type_reading catch-all used to
    # dequote the whole 'X and that Y' clause into a single PHANTOM fact type
    # (silent deontic loss). It must instead refuse LOUDLY (report unparsed),
    # leaving no junk 'X_and_that_Y' fact type in the schema.
    model = ("Explanation is an entity type. Hypothesis is an entity type. "
             "Explanation selects Hypothesis. "
             "It is forbidden that Explanation selects Hypothesis and that "
             "Explanation selects other Hypothesis.")
    D, _ = forml.compile_model(model)
    fts = [r[0] for r in system._pop_rows(D, "factType")]
    assert not any("and_that" in f for f in fts), fts     # no phantom
    assert "Explanation_selects_Hypothesis" in fts         # the real ft still compiles
