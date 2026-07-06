"""The abduction primitive, engine-side (whitepaper §3 + Theorem 4: induce is
a ρ-application over P answering candidate populations). Ported from the old
engine's induce.rs with its semantics as the oracle: candidates are the
cartesian product of each role's domain (declared enum values + the noun's
observed population; an empty domain collapses the product; an unknown fact
type answers []), gated by the fact type's alethic constraints as a BASELINE
DELTA (pre-existing violations never reject — the induce-baseline-delta-gate
lesson), optionally gated by forward-chain COVERAGE of to_explain, scored by
the app's Scoring Rules (the induction.md vocabulary: rules emitting
Hypothesis Candidate has Confidence Score rows; sum, numeric or 1 per
categorical row, 0 when none fire), ranked descending with enumeration order
stable on ties, and post-filtered by `bound` role pins."""
import pyarest.prims  # noqa: F401
from pyarest import apps


MODEL = """Person(.Name) is an entity type.
Room is a value type.
The possible values of Room are 'kitchen', 'library', 'study'.
Person was in Room.
Each Person was in at most one Room.
Person saw Person.
"""


def _mk(tmp_path, extra=""):
    root = tmp_path / "apps"
    d = root / "case" / "readings"
    d.mkdir(parents=True)
    (d / "app.md").write_text(MODEL + extra, encoding="utf-8")
    reg = apps.Registry(str(root), cache_dir=str(tmp_path / "fz"))
    reg.compile("case")
    return reg


def test_candidates_enumerate_enum_values_and_population(tmp_path):
    reg = _mk(tmp_path)
    reg.apply("case", "Person_saw_Person", ("Adler", "Moriarty"))
    hyps = reg.induce("case", "Person_was_in_Room")
    # persons observed anywhere x the declared rooms
    bindings = {(h["hidden"]["fact"][0], h["hidden"]["fact"][1]) for h in hyps}
    assert ("Adler", "kitchen") in bindings and ("Moriarty", "study") in bindings
    assert len(bindings) == 2 * 3
    ids = [h["id"] for h in hyps]
    assert ids[0].startswith("hyp-Person_was_in_Room-")


def test_unknown_fact_type_answers_empty_not_error(tmp_path):
    reg = _mk(tmp_path)
    assert reg.induce("case", "No_Such_Ft") == []


def test_the_alethic_gate_is_a_baseline_delta(tmp_path):
    # Adler is ALREADY in the kitchen: the at-most-one UC makes any second
    # room for Adler candidate-INTRODUCED — rejected. Moriarty stays open.
    reg = _mk(tmp_path)
    reg.apply("case", "Person_saw_Person", ("Adler", "Moriarty"))
    reg.apply("case", "Person_was_in_Room", ("Adler", "kitchen"))
    hyps = reg.induce("case", "Person_was_in_Room")
    people = {h["hidden"]["fact"][0] for h in hyps}
    assert "Moriarty" in people
    adler_rooms = {h["hidden"]["fact"][1] for h in hyps
                   if h["hidden"]["fact"][0] == "Adler"}
    assert adler_rooms <= {"kitchen"}                          # re-assertion only


def test_bound_pins_and_coverage_gates(tmp_path):
    RULE = ("Person1 is placed if Person1 was in some Room1.\n")
    reg = _mk(tmp_path, RULE)
    reg.apply("case", "Person_saw_Person", ("Adler", "Moriarty"))
    hyps = reg.induce("case", "Person_was_in_Room",
                      bound={"Person": "Adler"})
    assert {h["hidden"]["fact"][0] for h in hyps} == {"Adler"}
    # coverage: the candidate must forward-chain the observation
    hyps2 = reg.induce("case", "Person_was_in_Room",
                       to_explain=[{"ft": "Person_is_placed",
                                    "fact": ["Moriarty"]}])
    assert hyps2 and all(h["hidden"]["fact"][0] == "Moriarty" for h in hyps2)


def test_scoring_rules_rank_candidates(tmp_path):
    # the canonical Scoring Rule shape (the old engine's own): the app
    # declares the hidden hook fact type and rules over it; induce
    # materializes the synthetic hidden rows per candidate for the pass
    HOOK = "Hypothesis Candidate has hidden Room.\n"
    # the old engine's own spelling, verbatim shape — the IFF class-rule
    # ("... has Confidence Score '10' iff ... has hidden Side 'heads'").
    # (A numbered multi-word head leaks its subscript into the ft id, and
    # the unnumbered IF form compiles no rule — both noted as defects.)
    RULE = ("Hypothesis Candidate has Confidence Score '2' iff "
            "Hypothesis Candidate has hidden Room 'library'.\n")
    reg = _mk(tmp_path, HOOK + RULE)
    reg.apply("case", "Person_saw_Person", ("Adler", "Moriarty"))
    hyps = reg.induce("case", "Person_was_in_Room")
    assert hyps[0]["hidden"]["fact"][1] == "library"          # library outranks
    assert hyps[0]["confidence_score"] == 2
    assert hyps[-1]["confidence_score"] == 0                  # no rule fired
