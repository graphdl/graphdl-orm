# store-on-derive-default — Implementation Plan (2026-06-19)

> Board task: `store-on-derive-default` (p0, in_progress). Lever (claude app): `store-on-derive-default` (proposed). Closes defect `leaf-metamodel-non-idempotent-vs-recompile`.

**Goal:** replace the per-request drop-and-re-derive (#836) maintenance of materialized derived cells with **incremental view maintenance (IVM)**; add a genuine `pure` (never-materialized, recompute-on-read) opt-out. This is the asymptotic chain-cost lever and the root fix for leaf-vs-recompile non-idempotency.

**Sanction:** `AREST.tex:299` (Cost-model Remark) — append-only `P` ⇒ `lfp` admits materialized-view maintenance: insertion-only semi-naive (Bancilhon86) + the one controlled non-monotonicity (status projection) as classical IVM (Gupta93). Knaster–Tarski guarantees the cheap path agrees with the spec.

## Framing correction (load-bearing)
- `MaterializationPolicy` (`types.rs:1427-1444`) = `Stored` (`#[default]`) + `View`. **No `Pure`** / per-request-recompute concept exists.
- `Stored` (today's default) ALREADY materializes — the problem is the **maintenance**: drop-and-re-derive, not IVM. So "store-on-derive-default" = keep materializing by default, swap the maintenance to IVM, add `pure`.
- `View` (`*`) is NOT the opt-out — it's "stored AND lazily-resolvable" (still eagerly chained; `compile.rs:1978-2036`). `Pure` must mean genuinely never-materialized.

## Key finding: the IVM **insertion** primitive already exists
`semi_naive_inner` `delta_joins` path (`evaluate.rs:1072-1218`, default-on since 2026-06-16; this IS the `delta-occ-1…4` machinery — all completed). Given `initial_delta`, sidecar-complete rules evaluate over per-antecedent delta views `(ΔA⋈B)∪(A⋈ΔB)` — insertion-only semi-naive, **proven `delta==naive` on the sidecar-complete subset**. The apply path calls `forward_chain_defs_state_seeded_tracked` (no delta) instead → relies on wipe+replace. **Insertion half = wiring, not new math.**

## The keystone: the retraction wall
`merge_delta` (`ast.rs:6527`) + `merge_map_cell_contents` (`:6620`) **UNION only** (task-922) — a delta can add/overwrite-by-key but never *remove* a tuple. So a derived cell that should lose a tuple (e.g. a completed task leaving `Task_is_recommended`) can't be expressed as a delta; the apply path wipes the whole cell and force-replaces with the full recompute (`command.rs:3499-3543`). **The commit path must carry `(ΔD⁺, ΔD⁻)` and honor `ΔD⁻` — prerequisite for everything else.**

## Deletion / non-monotone scope (v1)
Confine negative deltas to (a) the SM current-status projection (already keyed last-write-wins, `evaluate.rs:575`) and (b) functional/keyed-antecedent overwrites (an `update` = `δ⁻+δ⁺` at one key). **General DRed (a tuple losing one of several supports) is OUT OF SCOPE v1** → fall back to scoped recompute for the sidecar-incomplete rule class (~64 rules, `evaluate.rs:1103`). SM-fold/trigger cells already self-correct — keep their drop-exclusion (`sm_family_consequent_cells`, `sm_trigger_cell_set`).

## Correctness invariant (the oracle)
`materialized_derived(apply_ivm(S0, m1..mn)) == derived(full_recompile(apply_base_only(S0, m1..mn)))` for any mutation sequence. Property test `ivm_after_n_mutations_equals_full_recompile` is the master gate — and it directly closes `leaf-metamodel-non-idempotent`.

## Gated plan (smallest-first, each TDD-gated & shippable)
- **Step 0 (no code):** write the oracle property test, wired to the CURRENT engine (passes today — everything drop-and-re-derives). Regression net for all later steps.
- **Step 1 (KEYSTONE — worktree-isolate):** retraction-capable commit. `ast.rs`: extend the delta to carry per-cell removals; `merge_map_cell_contents` removes `ΔD⁻` before unioning `ΔD⁺` (keep task-922 union for additions); `diff_cells` (`:6492`) emits `(added, removed)`. Test first: `merge_delta_removes_retracted_tuple_from_map_cell`, `merge_delta_retraction_then_reinsert_is_idempotent`. Extend the `merge_delta_is_inverse_of_diff_cells` receipt to removals.
- **Step 2 (gated by new `AREST_IVM` env, default-off):** route `transition_via_defs` (`command.rs:3030`) delta into `forward_chain_defs_state_seeded_with_delta`; skip the wipe (`:3359-3452`) + restore (`:3488-3554`). Test first: `transition_via_ivm_matches_drop_and_rederive`. Leave create/update on the old path.
- **Step 3:** extend to `create_via_defs`/`update_via_defs`; flip `AREST_IVM` default-on once all three pass the oracle on the real-metamodel corpus.
- **Step 4 (worktree-isolate):** converge load + `try_leaf_ingest` (`cli/entry.rs:1817`) + the load/full-compile wipes (`:3111-3158`, `load_reading_core.rs:1245`) on the IVM commit. Test `leaf_ingest_equals_full_recompile_with_retraction` (fails today, passes after) → closes `leaf-metamodel-non-idempotent`.
- **Step 5 (additive, independent):** `MaterializationPolicy::Pure` — enum + FT marker (`parse_forml2_stage1.rs:134-137`) + compile gate (emit only `view:{cell}`, exclude from `derivation:{id}`/`SyntheticDerivedCells`/`derived_wipe_set`) + read-path (`resolve_view`, `ast.rs:5941`). Test `pure_ft_is_never_materialized_and_resolves_on_read`.
- **Step 6:** retire dead code (`forward_reachable_consequents`, `derived_wipe_set`, snapshot/restore) once `AREST_IVM` stable; keep `AREST_IVM=0` as documented rollback.

## Key files
- `command.rs` — apply drop-and-re-derive: `create_via_defs:1326`, `update_via_defs:3797`, `transition_via_defs:3030`; `forward_reachable_consequents:2751`.
- `evaluate.rs` — fixpoint + IVM primitive: `semi_naive_inner:923`, delta-join `:1072-1218`, `_seeded_with_delta:879`, `integrate_round_facts:537`.
- `ast.rs` — commit/retraction wall: `merge_delta:6527`, `merge_map_cell_contents:6620`, `diff_cells:6492`; `resolve_view:5941`.
- `cli/entry.rs` — load drop `:3111-3258`, `derived_wipe_set:834`, `try_leaf_ingest:1730-1859`, `compile_input_sig:1451` (`_CompileSig` migration hook).
- `types.rs:1427-1444` — `MaterializationPolicy`; `compile.rs:1978-2036/4680-4727` + `parse_forml2_stage1.rs:134-137` — policy/markers.
- `load_reading_core.rs:1245-1278` — full-compile drop.

## Risks
- **IVM ≠ LFP → silent wrong derivations.** Gate: the Step-0 oracle, run with `AREST_IVM=1`; `AREST_IVM=0` rollback.
- **Retraction over/under-delete** for sidecar-incomplete (dynamic-read) rules → keep wipe+replace for that class until the `delta-joins-per-occurrence` lever lands.
- **Existing-DB migration** — first IVM apply must reconcile vs stored contents; one-time `full_recompile` on version bump via `_CompileSig`.

## Isolation note
Steps 1 and 4 touch the universal commit / cold-start paths AND collide with current uncommitted WIP in `ast.rs`/`evaluate.rs` — do them in a git worktree off a clean HEAD. Steps 2/3/5 are env-gated and safe in-tree.
