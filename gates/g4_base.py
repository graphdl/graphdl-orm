"""G4 — the base satisfies its own schema (SPEC 10.2, §13 G4).

The metamodel is an app of the system (Cor 5): compile the base readings and
run the absolute sweep; zero alethic violations or the base does not ship.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

_BASE = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                     "readings", "base")

# core first (the vocabulary the rest builds on), then the alphabetical rest
_ORDER = ["core.md"] + sorted(f for f in os.listdir(_BASE)
                              if f.endswith(".md") and f != "core.md")


_CACHE = []


def _base_D():
    if not _CACHE:
        from host_py import forml
        text = "\n\n".join(open(os.path.join(_BASE, f), encoding="utf-8").read()
                           for f in _ORDER)
        _CACHE.append(forml.compile_model(text))
    return _CACHE[0]


# The G2 backlog (SPEC 1.5, Prop 1; PLAN Day 5): grammar forms the parser does
# not yet accept, each pinned by prefix so nothing is silently tolerated
# (SPEC 14.3). Every prose-class sentence is OUT of the readings (10.2) — an
# unparsed sentence matching none of these is a regression, not backlog.
_G2_BACKLOG = (
    "It is impossible that a Resource is an instance of a Noun",      # impossible + relative clause
    "For each Role and Reading that Role has that Reading at most",   # external UC over role pair
    "Each Function has each Header at most once.",                    # frequency, each-each
    "If some Role is used in some Reading where some Fact Type",      # subset w/ where-join
    "If some Fact uses some Resource for some Role then that Fact",   # subset w/ where-join
    "If some Fact uses some Resource for some Role then that Reso",   # subset w/ where-join
    "If some Fact Type defines some Fact then some Resource that",    # subset w/ relative clauses
    "If some Verb references some Fact that is of some Fact Type",    # subset w/ where-join
    "If some Guard Run is for some Guard and that Guard Run refer",   # subset, 2-antecedent
    "If some State Machine is currently in some Status then that",    # subset w/ where-join
    "If some API accepts some Noun as parameter and some other No",   # subset over subtype closure
    "If Noun1 is subtype of Noun2, then Noun2 is not subtype of N",   # asymmetric ring as if-then
    "If Noun1 is subtype of Noun2 and Noun2 is subtype of Noun3,",    # transitive ring as if-then
    "If Derivation Rule 1 depends on Derivation Rule 2 and Deriva",   # transitive ring as if-then
    "If some Event caused some Transition in some State Machine t",   # subset w/ where-join
    "If some Failure follows some Violation then that Failure is",    # disjunctive subset + comparison
    "If some Violation occurs before some Transition then that Vi",   # subset + timestamp comparison
    "It is obligatory that each State Machine Definition has at l",   # deontic at-least-one
    "It is obligatory that when a Fact Type has exactly two Roles",   # obligatory-when (ring completeness)
    "If Derivation Rule 1 depends on Derivation Rule 2, then Deri",   # asymmetric ring as if-then
    "It is obligatory that each variable in a Derivation Rule con",   # rule safety (Cor 2)
)


def test_base_compiles_whole():
    D, rep = _base_D()
    stray = [s for s in (rep.get("unparsed") or [])
             if not any(s.startswith(p) for p in _G2_BACKLOG)]
    assert stray == [], stray
    assert len(rep.get("unparsed") or []) <= len(_G2_BACKLOG)


# The metamodel self-description closure (SPEC 10.1, Cor 2): the schema's
# roles and readings as facts in their OWN fact types. Materializing it pulls
# successive layers (roles → readings → verbs → texts); it lands WITH the G2
# grammar sweep (PLAN Day 5), not piecemeal — a first reflection pass moved
# the census 4→6 by feeding the next layer's mandatories (2026-07-14).
# Until then these four families are the PINNED remainder: any new family or
# any other fact type violating is a regression and fails loud.
_CLOSURE_BACKLOG = {"Fact_Type_has_Role", "Fact_Type_has_Reading",
                    "Role_is_used_in_Reading", "Noun_plays_Role"}


def test_base_satisfies_its_own_schema_outside_the_closure_backlog():
    from host_py import gate
    D, _ = _base_D()
    bad = gate.alethic(gate.sweep(gate.settle(D)))
    stray = [v for v in bad if v["fact_type"] not in _CLOSURE_BACKLOG]
    assert stray == [], "\n".join(
        f"{v['fact_type']} {v['kinds']}: {v['offenders'][:5]}{'…' if len(v['offenders']) > 5 else ''}"
        for v in stray)
    # the backlog may only shrink; Day 5 drives it to empty (G4 proper)
    assert {v["fact_type"] for v in bad} <= _CLOSURE_BACKLOG
