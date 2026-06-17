# claude-on-kernel: induce the live operational ledger from observations

**Date:** 2026-06-16
**Status:** Design — approved in dialogue; pending written-spec review
**Coordinated under:** `ledger-csdp-decomposition` (NOT a fork)
**Depends on / coordinates with:** `arest-kernel`, `arcmeta-3-derive-recommendation-from-planning`, `induce-coverage-gate-returns-empty` (the induce-verb coverage gate — the arc agent's prioritized engine p0)

## Problem & Goal

The operational ledger (`apps/claude`) records Engine Lessons / Engineering Levers / Measurements as facts, but their *substance* is still hand-authored: lessons are fact-*indexed* yet their finding lives in `# gloss` prose, and a new lesson requires a human to notice the pattern and write it. **Goal:** generate the ledger's lessons/levers as **induced derivation rules** read off observation facts — so the ledger is *live* (re-derived), *prose-free* (fact/rule references only), and *predictive* (flags new at-risk artifacts before they bite). This is the AREST AGI inner loop — induce operators → gate → propose → governed self-modification — applied to the engine's own operational memory.

## Key reframe (from the design dialogue)

The kernel (`arest-kernel`) is **not** a fixed shape library needing extension — its stratified meta-derivation-rules + CSDP `count==count` self-validation **are** the general inducer. The four shapes (`gather`/`relabel`/`majority`/`lookup`) are instances of one mechanism: `gather` literally *induces a feature→feature derivation rule* by cross-example consistency (`Feature1 gathers from Feature2 iff sourcematch Count == training total`). So inducing the ledger's lessons is **instantiating the existing design** (the kernel's intended "feed a domain by mapping its observations onto the I/O"), not a new mechanism. The stratified-meta-rule + CSDP design already covers conjunction→conclusion lesson rules; `majority` was itself added as "the next stratum," so the design grows by adding strata in that same style.

## Architecture

`claude-on-kernel` is a **consumer** of `arest-kernel` — same pattern as `maj-demo`: import the kernel; add a separate readings file mapping the ledger domain onto the kernel's I/O vocabulary. **No kernel changes** unless the existing shapes prove insufficient (deferred — see Sufficiency). Coordinated under `ledger-csdp-decomposition`; it does not fork a parallel task.

## The mapping (the core work)

Project the ledger's already-modeled engine-behavior facts (CSDP step 2, done) onto the kernel vocabulary:

- **Observation** = an Engine Cell (the artifact lessons are about).
- **Features** = the cell's behavior-ontology properties: `is-view-defined`, `is-stale-on-mutation`, `is-authoritative`, plus a relational feature `re-derived-by-a-disregarding-Code-Site` (derived from `Code Site re-derives Engine Cell` + `Code Site disregards materialized state`).
- **`Observation reads Value at Feature`** = the cell's observed property values; **`writes` / `must write` `is-defective`** = whether the cell is implicated in a `Defect` (from `Defect occurs at Code Site` / the cell facts).
- **Problem** = induce the feature-pattern that predicts `is-defective`, consistent across all observations.

## Output (dual-use)

The kernel self-validates (`count==count`) which shape/feature-pattern reproduces every known case, Sherlock-ranks the winners by confidence, selects one → the **induced rule**. Dual-use:
1. **Emits the Engine Lesson** — as a fact-referenced rule (concerns the Code Sites / Cells; carries a Lesson Kind), no prose.
2. **Predicts** — flags new Engine Cells matching the pattern as at-risk (the AGI payoff: it would flag the next staleness bug before it bites).

## MVP / worked example: the staleness class

Training set already on the board: the recurring SM-bridge-staleness defects (`924` / `955` / `task-status-bridge-blocked-lag` / `sm-ft-status-stage2-stale` / `recommendation-derivation-stale-on-mutation`) as positives; non-stale cells as negatives. Map them as Observations; run the kernel; expect it to induce the staleness rule (≈ *"a Cell that is `stale-on-mutation` AND `re-derived-by-a-disregarding-site` → `is-defective`"*), self-validated `count==count`.

## Admission + the induce-gate coordination

- **Admission (governance):** induced rule → `propose` → Domain Change SM (deontic human gate) → accepted rule runs live in the ledger.
- **Soundness gate (coverage):** the MVP uses the **kernel's** `count==count` coverage (declarative, in the kernel's meta-rules — working today, independent of the induce verb).
- **Coordination with the induce-verb gate:** the general/canonical induce-verb gate (enumerate → alethic-gate → forward-chain coverage → rank) is the arc agent's prioritized engine next-move — `induce-coverage-gate-returns-empty` (p0, pending: *"the to_explain coverage gate returns empty; no candidate is filtered as covering"*). claude-on-kernel's declarative coverage **shares semantics with that gate, and must not fork a second divergent one**. The richer induce-verb-gated path sequences **after** that engine fix; the kernel-declarative MVP does **not** block on it.

## Sufficiency of the existing shapes (deferred — "cross the bridge if reached")

The four existing shapes each fix a single relation form; a lesson generalizes over a conjunction-**subset** of features. Whether the existing shapes (especially `lookup`'s whole-context agreement) already induce the staleness rule, or a new conjunction-lesson stratum is needed, is left to **empirical discovery in the MVP**. If a stratum is needed, it is written in the same CSDP `count==count` style (the kernel's normal growth, as `majority` was added).

## Dependencies

- **`arest-kernel`** — the induction substrate the mapping consumes.
- **`ledger-csdp-decomposition`** — the fact-reference schema for Lessons/Levers (done; the induction targets it). This work is coordinated *under* it.
- **`arcmeta-3-derive-recommendation-from-planning`** — arc-meta's induce+planning meta-rules; shared machinery.
- **`induce-coverage-gate-returns-empty`** — the induce-verb coverage gate (arc-prioritized engine p0); coordinate semantics; the full induce-verb path depends on it.

## Testing

- **`count==count` self-validation:** the induced rule reproduces ALL known staleness defects (positives) and none of the negatives.
- **Held-out:** a known `stale-on-mutation` cell withheld from training is predicted `is-defective` by the induced rule.
- **No-prose check:** the induced lesson is pure fact/rule references (no `Lesson Statement`, no gloss carrying substance).

## Out of scope

- The conjunction-lesson stratum (deferred; only if the MVP shows the existing shapes insufficient).
- The 3 governance Operating Rules' prose `Rule Statement` (separate `ledger-csdp-decomposition` cleanup).
- The induce-verb gate fix itself (`induce-coverage-gate-returns-empty` — the engine agent's task; we coordinate, not duplicate).
