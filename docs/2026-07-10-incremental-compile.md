# Incremental compile: freeze the base's derived state, pay only the app delta (#20)

Design record, 2026-07-10. Host-independent; the Rust CLI is the intended first
implementation, the Python engine is the reference.

## The measured problem

A **null app** (`identity`, one reading, `unparsed=2` — nothing compiles) still costs
**56s compile + 40s validate**, warm base already in memory. The phase breakdown (Python
reference, warm base thaw 0.33s):

| phase                     | time  | what it does |
|---------------------------|-------|--------------|
| create_handlers           | 13.6s | rebuilds a `create:<ft>` handler for **every** fact type, base+app |
| run_rules (post-model)    | 12.6s | **full** fixpoint over base+app — called with no `changed=` |
| sql-project               | 11.9s | re-projects the entire base to relational tables |
| layout+scheduler+generator| 7.2s | recomputes base layout/schedule/generator cells |
| machine_fold              | 3.5s  | folds machine events over base+app |
| compile_model (g-loop)    | 3.7s  | the translator loop — the small slice |
| status_facts              | 2.1s  | re-derives status columns |

The g-loop is 3.7s of 56s. The cost is the **derive pipeline re-deriving frozen-invariant
base state** on every app compile. `ingest_frozen` (protocol.py) freezes only
`compile_model(text)[0]` — base facts and rule DEFs — **not** the derived closure, handlers,
SQL projection, or layout. So they are recomputed from scratch each time.

## The fix

"Schema in memory" must mean the base's **derived** artifacts are frozen alongside its facts.
Then an app compile is: thaw fully-derived base (fast) -> `compile_model` the app delta ->
run each derive phase **incrementally on that delta only**.

### 1. Freeze the derived base

Run the full pipeline on the base ONCE, at base-compile time, and freeze the result:
`compile_model` -> `run_rules` (full) -> `status_facts` -> `create_handlers` ->
`layout/scheduler/generator` -> `sql-project`. The frozen sidecar then carries the closure,
the `create:<ft>` handler family, the projected tables, and the layout cells. This makes the
COLD base build pay once (cached by engine fingerprint, as today); every later app compile
thaws it.

### 2. Delta each phase against the app's changes

`compile_model` returns the set of cells the app touched (`Δ`). Each phase consumes `Δ`:

- **run_rules(changed=Δ)** — semi-naive derivation from the frozen base closure, not a full
  re-fixpoint. Already supported: Python `run_rules(D, changed=...)` (engine.py:1200) and the
  Rust resident `op_run_rules` both take `changed`. The compile just has to hand it one.
- **create_handlers(only=Δ_factTypes)** — build handlers for app-introduced/affected fact
  types; reuse the frozen base handlers untouched (today it strips ALL `create:` cells and
  rebuilds every one).
- **sql-project(only=Δ_tables)** — project the app's tables; the base tables are frozen.
- **status_facts / machine_fold / layout** — delta-aware, or skip when `Δ` touches nothing in
  their domain (the null-app case pays ~zero).

### 3. Correctness

The base's derived state is app-independent: base rules fire on base facts, producing a closure
that does not depend on any app. An app adds facts; semi-naive `run_rules(changed=Δ)` fires only
the rules reachable from `Δ` against the frozen closure — the standard delta-derivation, and set
semantics make it idempotent. Handlers and projections are per-fact-type/table, so they compose
additively. The end state is identical to a full recompile; only the wasted base re-derivation
is removed.

## Why host-independent (not a Python tweak)

This is a compile-**strategy** change — what gets frozen and what gets recomputed — not a
reducer micro-optimization. Every host that compiles benefits, and the freeze boundary +
delta-tracking live in the compile orchestration (the host driver), above the canon. The canon
DEFs each phase reduces are unchanged; they are simply reduced over `Δ` instead of over base+app.
Pairs with the canon-level algorithmic fixes (e.g. `ast:Pop` O(n^2)->O(position), 4bcbdbcf):
canon makes each reduction cheap, incrementality makes the reductions few.

## Expected

App compile: `56s` -> base thaw (`0.33s`) + app delta (seconds). Validate: same treatment —
freeze the base's satisfied constraints, re-check only constraints whose populations `Δ` touched.

## Open

- Threading `Δ` out of `compile_model` and into each phase (the phases currently take only `D`).
- The freeze format grows to hold derived cells; invalidation stays by engine-fingerprint +
  text hash (a changed base or engine is a new sidecar, as today).
- Sequencing against replay: replayed event facts are another `Δ` batch through the same
  incremental phases.
