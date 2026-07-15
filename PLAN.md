# The build plan

Work items key to SPEC clauses and gates. One-shot per D5: no review pauses; a step
that cannot keep the gates green is reverted, not patched around (SPEC §13).

## Layout

    /arest.canon        the shared codebase (8.1, D7)
    /readings/          metamodel readings, sentences of R only (10.1, 10.2)
    /apps/<name>/readings/   conformance apps: paper-order, organizations,
                             redo-decision (G3)
    /host-py/           Python μ-evaluator (§9)
    /host-rs/           Rust μ-evaluator (§9)
    /gates/             g1..g7 runnable (§13)
    /SALVAGE.md         the ledger (14.3)

## Day 1 — constitution (done)

SPEC.md + AREST.tex, root commit 7b48bd86.

## Day 2 — the canon and G1

2.1 Copy arest.canon from the quarry (canon-first 0fa14b7c, engine/shared/) to
    /arest.canon. Ledger entry.
2.2 Salvage the ρ-fidelity litmus (quarry engine/tests/test_tromp.py) to
    /gates/g1_litmus.py, freed of quarry imports.
2.3 Enumerate the 17 boundary DEFs (362 total, 345 pure-closed at the quarry).
    Each becomes a declared Def-9 registered tuple in the canon's manifest or is
    rewritten pure (8.2). G1 = all 362 accounted.

## Day 3 — the Python μ-host

3.0 (moved from Day 2) Salvage metamodel readings, scrubbed: prose-as-name out
    (10.2), world assumptions defaulted open (3.4) — gated by the Day-3
    compiler, not copied blind (14.2).

3.1 host-py kernel: μ over the H1 forms, cells (H2). Salvage quarry kernel where
    litmus-clean.
3.2 The single transition (H3) and command stages (2.1); THE ONE GATE (2.2):
    checkers enumerated from the compiled store, never listed in host code.
3.3 Compiler: parse on fragment R (1.5), two-family constraint compilation (1.6),
    RMAP cells (6.1), verbalize with nf idempotence (Prop 1). Salvaged behaviors
    enter only through G2.
3.4 retract (2.5), batch steps (2.4), journal + replay (§12).
3.5 G4 base self-validation. (G7, the Cor-2 acyclicity query on ingest,
    moves to Day 5/6 with the rule-grammar work — the ingestion path and
    rule read/derive machinery are touched together there.)

## Day 4 — the Rust μ-host

4.1 host-rs: include! the same canon bytes; μ, cells, dispatch, the same gate.
4.2 G3 same-bytes over the conformance apps.

## Day 5 — G2 and salvage completeness

5.1 NORMA corpus sweep to zero unparsed, nf ∘ nf = nf.
5.2 SALVAGE.md completeness: every quarry artifact class dispositioned or listed
    with its rejection (14.3).

## Day 6 — representation and self-gating

6.1 links/emit (Thm 2), completeness as a test.
6.2 MCP transport adapter forming SYSTEM:x (5.2) — zero meaning in the adapter.
6.3 Self-gating turns: a turn is a step, judged by the gate (Cor 2, Cor 5).
6.4 G5 and G6 regressions scripted and green.

## Day 7 — buffer

Full gate run. Main-replacement proposal drafted, NOT executed (D5).
