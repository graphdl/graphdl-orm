"""Verdict eleven's remaining analytics family, isolated to engine shapes: a
unary atom BETWEEN two binaries must not break variable unification across
them (the claude Agenda_ranks rule derives 20 rows where the corpus's own
cells join to 5), and a TWO-word role qualifier ('worst total', where 'peak'
and 'base' are one word) must still compile the aggregate head. The old
snapshot is self-consistent on both: these are OUR compile defects, proven
against the corpus's own data."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system


def _derive(model):
    D, rep = forml.compile_model(model)
    assert rep["unparsed"] == []
    return system.run_rules(D), rep


def test_a_middle_unary_atom_keeps_cross_atom_unification():
    # the Agenda_ranks shape (solver-loop.md:71): considers(A, SR) x
    # actionable(EL) x has_rank(EL, SR) — SR must unify across atoms one and
    # three, THROUGH the width-one atom between them
    model = """Agenda is an entity type.
Engineering Lever is an entity type.
Stratum Rank is a value type.
Agenda considers Stratum Rank.
Engineering Lever is actionable.
Engineering Lever has Stratum Rank.
Agenda ranks Engineering Lever at Stratum Rank. *

* Agenda1 ranks Engineering Lever1 at Stratum Rank1 iff Agenda1 considers Stratum Rank1 and Engineering Lever1 is actionable and Engineering Lever1 has Stratum Rank1.

Agenda 'current' considers Stratum Rank '1'.
Agenda 'current' considers Stratum Rank '4'.
Engineering Lever 'e1' is actionable.
Engineering Lever 'e2' is actionable.
Engineering Lever 'e1' has Stratum Rank '1'.
Engineering Lever 'e2' has Stratum Rank '2'.
Engineering Lever 'e2' has Stratum Rank '4'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(str(x) for x in r) for r in system._pop_rows(
        D, "Agenda_ranks_Engineering_Lever_at_Stratum_Rank")}
    # e1 at rank 1 (considered), e2 at rank 4 (considered; its rank 2 is not)
    # — NOT the cross product of considered ranks with actionable levers
    assert got == {("current", "e1", "1"), ("current", "e2", "4")}


def test_a_two_word_role_qualifier_compiles_the_aggregate_head():
    # the App_has_worst_total_Duration_Ms shape (compile-perf.md:79): 'worst
    # total' qualifies Duration Ms with TWO words, where 'peak' and 'base'
    # are one — the head must still parse and the max must still fold
    model = """App is an entity type.
Duration Ms is a value type.
App has run total Duration Ms.
App has worst total Duration Ms. *
Each App has at most one worst total Duration Ms.

* App1 has worst total Duration Ms iff Duration Ms is the max of Duration Ms1 where App1 has run total Duration Ms1.

App 'a' has run total Duration Ms '7'.
App 'a' has run total Duration Ms '3'.
App 'a' has run total Duration Ms '9'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    got = {tuple(str(x) for x in r) for r in system._pop_rows(
        D, "App_has_worst_total_Duration_Ms")}
    assert got == {("a", "9")}


def test_max_folds_numerically_over_sum_derived_integers():
    # compile-perf.md:41,79: total is a SUM (integers, via the arithmetic
    # coercion), and worst folds max OVER those integers — the comparator must
    # order what arithmetic produced, or the fold bottoms and worst derives
    # NOTHING (the corpus verdict: worst_total 1v0)
    model = """App is an entity type.
Compile Run(.id) is an entity type.
Compile Phase is a value type.
Duration Ms is a value type.
Compile Run profiles App.
Compile Run spends Duration Ms in Compile Phase.
Compile Run has total Duration Ms. *
App has run total Duration Ms. *
App has worst total Duration Ms. *

* Compile Run1 has total Duration Ms iff Duration Ms is the sum of Duration Ms1 where Compile Run1 spends Duration Ms1 in Compile Phase1.

* App1 has run total Duration Ms iff Compile Run1 profiles App1 and Compile Run1 has total Duration Ms.

* App1 has worst total Duration Ms iff Duration Ms is the max of Duration Ms1 where App1 has run total Duration Ms1.

Compile Run 'r1' profiles App 'tasks'.
Compile Run 'r2' profiles App 'tasks'.
Compile Run 'r3' profiles App 'tasks'.
Compile Run 'r1' spends Duration Ms '300' in Compile Phase 'parse'.
Compile Run 'r1' spends Duration Ms '5' in Compile Phase 'derive'.
Compile Run 'r2' spends Duration Ms '90' in Compile Phase 'parse'.
Compile Run 'r2' spends Duration Ms '9' in Compile Phase 'derive'.
Compile Run 'r3' spends Duration Ms '1500' in Compile Phase 'parse'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    total = {tuple(str(x) for x in r) for r in system._pop_rows(
        D, "Compile_Run_has_total_Duration_Ms")}
    # r3's sum is a SINGLETON: Backus INSERT over one element answers the
    # element unapplied, so the cell MIXES the string '1500' with int sums —
    # the live claude store's exact shape (cold-sp1 '11000' beside 4997)
    assert total == {("r1", "305"), ("r2", "99"), ("r3", "1500")}
    worst = {tuple(str(x) for x in r) for r in system._pop_rows(
        D, "App_has_worst_total_Duration_Ms")}
    # a string-only comparator handed the mixed cell bottoms the whole fold
    # (the corpus verdict: worst_total 1v0); coerced, 1500 wins numerically
    assert worst == {("tasks", "1500")}


def test_max_and_min_coerce_multi_digit_text_durations():
    # compile-perf.md:40: peak folds over ASSERTED text durations — lexical
    # max picks '305' over '1190' (the corpus verdict: peak content diffs);
    # numbers stored as text order numerically wherever they parse, exactly
    # like the arithmetic coercion
    model = """Compile Run(.id) is an entity type.
Compile Phase is a value type.
Duration Ms is a value type.
Compile Run spends Duration Ms in Compile Phase.
Compile Run has peak Duration Ms. *
Compile Run has floor Duration Ms. *

* Compile Run1 has peak Duration Ms iff Duration Ms is the max of Duration Ms1 where Compile Run1 spends Duration Ms1 in Compile Phase1.

* Compile Run1 has floor Duration Ms iff Duration Ms is the min of Duration Ms1 where Compile Run1 spends Duration Ms1 in Compile Phase1.

Compile Run 'r1' spends Duration Ms '305' in Compile Phase 'parse'.
Compile Run 'r1' spends Duration Ms '1190' in Compile Phase 'derive'.
Compile Run 'r1' spends Duration Ms '22' in Compile Phase 'emit'.
"""
    D, rep = _derive(model)
    assert rep["rule_diagnostics"] == []
    peak = {tuple(str(x) for x in r) for r in system._pop_rows(
        D, "Compile_Run_has_peak_Duration_Ms")}
    floor = {tuple(str(x) for x in r) for r in system._pop_rows(
        D, "Compile_Run_has_floor_Duration_Ms")}
    assert peak == {("r1", "1190")}
    assert floor == {("r1", "22")}
