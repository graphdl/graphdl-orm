# Mining the old ~6s Rust compile (wt-orm2-conformance) — transplant map (#20/#25)

2026-07-10, agent-mined with citations from the branch. compile-perf.md does not exist in
history; timings reconstructed from the branch's own instrumentation comments.

## Headline: the 6s was native-FIRST, rayon-SECOND

1. ALL-NATIVE, zero process hops: readings -> parse_to_state -> compile_to_defs_state
   (five profiled phases: constraints, state-machines, noun-index, derivations, schemas,
   each lowered ONCE to a Backus Func) -> defs_to_state -> persist. Native compile
   ~500ms-1.3s (entry.rs:1570/2488). The phases that cost main 13.6/12.6/11.9s in Python
   are each native sub-second. create_handlers has NO separate phase — the handler Funcs
   (resolve:/query:/schema:/validate:/derivation:) are emitted DURING compile.
2. RAYON as a multiplier: par_iter over EXACTLY three combining forms in the evaluator —
   Construction (ast.rs:3081, threshold >=16), ApplyToAll (3114, >=64), Filter (3170,
   >=64). Behind a default-on `parallel` cargo feature (Cargo.toml:372/405/477).

## The parallelism architecture (the #25 corrections)

- Indexed par_iter().map().collect() into ordered Vec -> Object::seq: output order =
  input order regardless of schedule. Maps are PURE over the shared immutable store &d —
  no locks, no merge inside the parallel region. Merge is SERIAL between forward-chain
  rounds (evaluate.rs:1031/1063).
- FUEL GATE (the elegant part): every parallel branch is gated on !fuel_is_bounded()
  (ast.rs:3082/3115/3171; thread-local APPLY_FUEL, ast.rs:136). Bounded evaluation stays
  serial; only unbounded (compile/load/derive) fans out. Parallelism and fuel-bounded
  termination are mutually exclusive by construction. main's NEval.fuel already uses the
  <0 = unbounded sentinel (main.rs:1317) — reuse directly.
- INSERT/fold is DELIBERATELY NOT parallel (ast.rs:3128-3159): recursive fold overflowed
  rayon worker stacks on bulk derivations; converted to an iterative O(1)-stack right
  fold and found fast enough. #25's INSERT tree-reduction is DROPPED on this evidence
  (or bounded-depth over iterative chunks only if serial ever measures short).
- NO per-rule parallelism: the rule loop is serial, which SIDESTEPS the concurrent
  store-merge problem entirely. #25's per-rule idea is DEFERRED: land the alpha-arm
  first (proven), measure, only then consider per-rule.
- Verdict on #25: CONFIRMS rayon-the-alpha-arm (and says also do Filter + Construction,
  thresholds 64/16); CONFIRMS carrier-must-be-Send (old Object is Send+Sync); DROPS
  INSERT tree-reduce; DEFERS per-rule. Landing site: the NEval combinator arms
  (main.rs:1320-1352+) + reduce_over (2822), driven from op_run_rules (3134).

## Pipeline techniques main's op_run_rules lacks (derive)

1. Semi-naive delta-joins with per-cell delta views, soundness-gated by
   derivation_reads_complete markers (evaluate.rs:1002-1030; compile.rs:5410-5422).
2. DIRTY-CELL ACTIVATION GATING with per-rule antecedent read-set sidecars
   (evaluate.rs:1047-1078; compile.rs:5326-5409) — the ~19s -> seconds technique.
3. Incremental key maintenance: existing_keys HashSet built once, updated per round
   (evaluate.rs:1039-1046).
4. Keyed-cell routing (read_cell_key_roles / cell_put_keyed upserts).
5. IndexBy hash-join combinator (ast.rs:3237) — O(n) equi-joins vs nested loops.
Plus a per-round wall-clock deadline with traced-bottom culprit naming (evaluate.rs:1061).

## Parser/translators (the #18 Rust half)

- Classification = forward-chain over the GRAMMAR's rules (parse_forml2_stage2.rs:2289)
  — the same doctrine as main's classify_all_via_M, already native.
- 13 COARSE hand-written translators (parse_forml2_stage2.rs:246-259) dispatched via a
  table built FROM the Classification_has_Translator cell (:2052-2087) — the identical
  grammar relation main keys on. 24 kinds -> 13 fns (families share: cardinality = UC+MC+
  Frequency; set = Subset+Equality+Exclusion+XO+Or). A model for how FEW native fns
  main needs. Under the certified-twin doctrine these become performance twins with the
  canon DEFs as oracle — re-expressed, not discarded.

## SQL projection / handlers

- RMAP pure native: rmap_from_state -> Vec<TableDef> (rmap.rs:181/941-944),
  create_table_sql (1455), ONE projection_plan (entry.rs:115/376) drives the read verb
  AND persist (children-first DELETE + parents-first INSERT in Kahn order,
  entry.rs:333-377). Kills main's 11.9s Python sql-project as a Low-Med transplant.
- machine_fold = the SM compiled to a Func fold, run by the same evaluator.

## The transplant ranking (impact-for-effort)

| # | technique | effort | kills |
|---|---|---|---|
| 1 | rayon alpha/Filter/Construction arms (fuel-gated, thresholds) | Med | single-file evaluation (#25) |
| 2 | native RMAP + projection_plan -> op_sql_project | Low-Med | the 11.9s Python phase |
| 3 | dirty-cell activation gating + read-set sidecars | Med | derive re-fire waste (19s->s) |
| 4 | the 13 coarse translators as certified twins | High | the cook/translator Python residue (#18's Rust half) |
| 5 | key interning + IndexBy hash-joins | Low-Med | per-round re-hash + nested-loop joins |

Caching notes: the old metamodel OnceLock cache + load-state sidecar are PROCESS-LOCAL
METAMODEL caches — distinct from the derived-base freeze main tried and REJECTED; any
transplant is framed metamodel-only.
