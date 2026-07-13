# Incremental aggregates — the seconds-class lever (#35, #20)

2026-07-12. The `aggfinal` gate proved native aggregate *execution* is byte-clean but
**wall-neutral** (tasks-evt 3,417 s vs 3,283 s native-rules baseline). The aggregate
wall is not execution-speed-bound; it is **algorithm-bound**: the priority-min
aggregate rules recompute their whole result every fixpoint round. This spec is the
algorithmic fix — a host-side *fast override* of the fixpoint's aggregate pass,
certified byte-identical to the canon's full-recompute. The canon keeps
`system:compile_agg_rule`; the host changes only *how much* it re-evaluates.

## Priority note (2026-07-12): this is a SECONDARY lever, not the primary

The parallel audit + the profiler in `docs/2026-07-10-rust-native-compile.md` reset the
seconds-class priority. The PRIMARY lever is the **Rust-native compile** (fast host
reducer over the canon): `apps_compile`/`compile` still delegate to Python
(main.rs:12785), the compile is combinator-bound (~23M already-native-twin primitive
calls dominated by Python-interpreter overhead — no single-op win to chase), and the
native `op_compile_model` is a skeleton gated on completing #18 (a doctrine-governed
Stage-1/translator boundary refactor, not a DEF copy). Wiring the native compile is both
the "compile without Python" requirement and the main perf win. THIS incremental-
aggregate lever is a secondary, within-`run_rules` win for big-app stress (tasks-evt's
3,417 s) — pursue it AFTER the native compile is wired, not before. Kept here as the
scoped design for when its turn comes.

## Target validated (2026-07-12)

The tasks-evt gate times `op = "compile_model"` (stage-e-run.ps1, explicitly the
"Python-free" harness) — the NATIVE store-twin compile, not the Python-delegated
`apps_compile`. `compile_model` = rekey + `run_rules` (the derive) + rep, so the
`run_rules` aggregate recompute IS on the timed path. The 3,417 s wall is native-Rust
cost; this lever attacks the right thing. (The separate Python-delegation audit of the
ops surface is task #29 — orthogonal to this perf lever.)

Also noted: a delta-eval path exists — `FastStore::eval_delta(nprocess, variant, drows_n)`
(main.rs:3814) reduces a rule's `~d` variant over explicit delta rows. VERIFIED
(2026-07-12): the `~d` variants are **plain-rules-only** — they drive the semi-naive
inner join for conjunctive rules (main.rs:4382-4390, `"<rid>~d<pos>"`); the `agg` pass
(4754-4769) uses `eval_full` over whole `D` and carries no delta variant. So the
monotonic-merge-via-`eval_delta` route is NOT available for aggregates without compiling
new variants. The **restricted-`D` recompute** (below) is the approach, and its sub-O(N)
form needs an **IndexBy** (group-key → rows index on the aggregate input, maintained
incrementally) — which is exactly the second half of #35's title ("frontier the post-fold
derive + IndexBy round one"). Full scope: round_delta capture → Δkeys → IndexBy fetch of
Δkeys groups → fold → the existing per-group supersession (4806). This is a focused,
multi-part byte-critical build, not a patch.

## Where the cost is (main.rs `op_run_rules`, the `agg` pass 4753–4805)

Per round, for each aggregate rule `rr`:

- **Rule-level frontier gate — already present** (4758–4768): if `outer > 0 ||
  dirty.is_some()`, the rule fires only when some cell it `reads` is in `dirty` or
  this round's `round_changed`. Aggregates whose inputs were untouched are skipped.
- **The cost — `eval_full` (4769)**: when the rule *does* fire, `store.eval_full(rid)`
  reduces the aggregate body over the **whole population** — every group's min/count,
  even if a single group's members changed.
- **Per-group supersession — already present** (4806–4813): on a frontier round the
  produced groups replace their stored rows and groups no rule produced **survive**
  from `before_rows`. So the *merge* is already incremental; only the *compute* is whole.

So `eval_full` computes all G groups when only Δ ⊆ G changed. Cost ≈ rounds × G ×
per-group work; the fix makes it rounds × |Δ| × per-group work, |Δ| ≪ G.

## The fix — recompute only the changed groups

Option chosen (correct regardless of monotonicity, and it reuses the existing
supersession merge): **evaluate the aggregate body over the population restricted to
the changed groups**, yielding those groups' full aggregate values; unchanged groups
survive via 4806. Byte-identical because a changed group's restricted recompute equals
its whole-population value, and untouched groups are exactly `before_rows`.

Steps:

1. **Changed group keys this round.** IMPORTANT (verified 2026-07-12 reading the loop):
   the fixpoint passes do **not** use the `~d` / `eval_delta` row-level path — every
   pass (`agg` 4769, `keyed` 4848, `sweep`/`dred` 4895) calls `eval_full` /
   `eval_rules_many` over the settled store, gated only at **cell** granularity
   (`touched_by`, `round_changed` holds head *names*). So there is no ready row-delta to
   read. But each pass already *computes* its own row delta to decide `same` — e.g. the
   `agg`/`keyed` merge diffs `merged` against `before_rows`/`stored` (4816, 4871). The
   plumbing is therefore: **capture the added rows per changed cell** (`merged` \
   `before`, keyed by cell) into a `round_delta: HashMap<cellkey, Vec<row>>`, then a
   fired aggregate reads `round_delta` for the cells it depends on and projects those
   rows to the head **group key** (key-span = every head column but the last, per
   `group_key`, main.rs ~3809). That set `Δkeys` is the groups to recompute. No new
   `~d` variants needed — the delta is a by-product the loop already forms.
2. **Group-scoped eval.** Add an eval mode `eval_agg_groups(rid, Δkeys)` that runs the
   compiled aggregate body but filters the grouped population to `Δkeys` before the
   INSERT-fold (inject a `theta:Filter(group ∈ Δkeys)` ahead of the ALPHA group-select
   in `compile_agg_rule`, or filter the fetched atom rows by group membership). It
   returns only the `Δkeys` groups' rows.
3. **Merge = the existing supersession** (4806): produced (`Δkeys`) groups replace;
   all others survive. No new merge code.
4. **Round zero / full derive** (`dirty.is_none()`, the `aggwhole` whole-replace at
   4790) stays `eval_full` — the first build must be whole. Only frontier rounds
   (`outer > 0 || dirty.is_some()`) take the group-scoped path.

Monotonicity note: the least-fixed-point derive only *adds* facts, so an even cheaper
merge (min = min(old,new), count += Δ, sum += Δ, max = max(old,new)) is valid; but the
restricted-recompute above avoids the retraction hazard (a removed min) entirely and is
the safer first cut. Keep the monotonic merge as a later refinement.

## Certification (non-negotiable — same discipline as the twin passes)

- **Kill switch** `AREST_NO_INCR_AGG=1` forces `eval_full` (the differential oracle).
- **Fixture first** (fast: the `agg-on-dbg` gate ran in 1.63 s): incr-on vs incr-off
  must produce **byte-identical final stores** over the fixture and kernel/sherlock
  corpora — reuse `stage-e-run.ps1` / `fleet-twin-cert.ps1`.
- **Gate**: tasks-evt wall incr-on vs the 3,283 s baseline, byte-clean first. Run it on
  a **quiet machine** (the 3,417 s aggfinal was contaminated by concurrent load — do
  not repeat that; no arest-show, no builds, no probes during the timed run).

## Scope / risk

Bounded: one new eval mode + a group-key delta projection + reuse of the existing
supersession. It does not touch `compile_agg_rule` (the canon aggregate stays the
definition) — it is a registered fast path for the fixpoint, exactly the "fast override
per platform" the standing directive calls for, gated by a kill switch to the canon
oracle. Expected payoff scales with how few groups change per round; the priority-min
aggregates over a large task population are the target case where |Δ| ≪ G.

## Sequencing

1. Add `eval_agg_groups` + the `Δkeys` projection; wire it into the `agg` pass frontier
   branch (main.rs 4769, guarded by the kill switch and `dirty.is_some()`).
2. Fixture byte-cert (fast) → kernel/sherlock byte-cert.
3. tasks-evt wall gate on a quiet machine.
4. Then the monotonic-merge refinement and α-parallelism (#25) over the per-group folds.
