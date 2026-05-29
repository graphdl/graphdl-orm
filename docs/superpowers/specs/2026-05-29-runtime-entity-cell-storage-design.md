# Runtime Entity‑Cell Storage Rework — Design

**Date:** 2026‑05‑29
**Status:** Draft for review
**Scope:** AREST engine core — runtime population storage (local/SQLite host first)
**Sources:** `arest.tex` (whitepaper), `infosci/` (Halpin reference library), `crates/arest/src/rmap.rs`, `docs/11-portability.md`, `docs/12-physical-mapping.md`

---

## 1. Problem: the storage inversion

The whitepaper fixes the physical model precisely:

- **Def. Cell Isolation** (`arest.tex:198‑202`): *"RMAP assigns each entity to its own cell whose fold is independent."*
- **`arest.tex:456`**: *"Each cell stores the 3NF row."*
- **eq:demux** (`:458`)  `Eₙ = Filter(eq ∘ [RMAP, n̄]) : E`
- **eq:cellfold** (`:462`)  `Dₙ' = foldl μₙ Dₙ Eₙ`
- **eq:pop** (`:466`)  `P = ⋃ₙ ↑FILE : Dₙ`

Read together: the **stored cell `Dₙ` is one entity's 3NF row**; the elementary‑fact population **`P` is *derived* from the cells by ↑FILE** (reconstitution). RMAP assigns each entity its own cell; `μ` folds events into that cell.

**The local runtime store does the inverse.** It stores the *elementary fact‑type* cell — `Task_has_Task_Description` holds *every* task's description — serialized as an `Object` string blob in `cells(name, contents)`, and treats the 3NF entity row (`get:Task`) as a derived view. It stores `P` and derives `Dₙ`, the opposite of eq:pop.

This is drift, not the design. Evidence the canonical model is already in place elsewhere:

- **Metamodel already uses entity cells.** The `Noun` cell stores each noun as a 3NF row with `objectType`, `referenceScheme`, `enumValues` as **absorbed columns** (`rmap.rs:331‑339`), not as separate `Noun_has_ObjectType` fact cells.
- **Cloud already uses entity cells.** One Durable Object per cell, named `(nounType, entityId)` → `Task:909`, each holding the entity's 3NF row (`docs/12-physical-mapping.md`; `src/api/cell-key.ts`).
- **The engine already computes the mapping.** `rmap.rs` is textbook Halpin Ch. 10 (entity = table, functional roles absorbed as columns, M:N/ternary → own table); `rmap_cell_map` (`rmap.rs:794`, "paper Eq. demux") classifies each FT to its owning entity cell; `EntityCellRouter` (`rmap.rs:1046`, "per‑entity cell routing, paper §196 + §462 eq:cellfold") routes a fact into `<Noun>:<id>` and pulls the `(field, value)` pair; `reconstitute_absorbed_ft` (`rmap.rs:919`) *is* ↑FILE ("the up‑FILE reconstitution of eq:pop").
- **It is a known follow‑up.** `rmap.rs:918`: *"Runtime per‑entity cells (`<Noun>:<id>`) are a follow‑up."*

### 1.1 The bug classes are all downstream of the inversion

| Bug class | Why it exists | Removed by storing `Dₙ` (entity rows) |
|---|---|---|
| Escaping corruption (`escape_atom_for_display`/`parse`, `ast.rs:416‑557`) | The blob codec serializes `Object` cells to a single TEXT string and re‑parses; delimiters in values (`= { } < ,`) collide with FFP syntax | Values live in typed columns, bound as SQL parameters and read back verbatim — there is no string codec to corrupt |
| `_CellKeyRoles` keyed‑fold collapse (`compile.rs:3176‑3225`, `evaluate.rs:90‑134`) | FT‑keyed cells need an out‑of‑band registry telling the fold which roles to key each cell by; it is re‑derived from round‑tripped constraint spans and emits empty on recompile (787→1) | The cell **is** the entity, keyed by its reference scheme = the table PRIMARY KEY. Key roles are table metadata, never a persisted registry |
| Preserve / merge / dedup / orphan‑GC dance (`ast.rs:5160‑5193`, `:4850‑4870`, `cli/mod.rs:149‑201`, `ast.rs:5037‑5061`) | Schema recompile reparses readings into a fresh FT‑cell state, then must reload the prior population blob, splice it onto the fresh schema, and re‑clean accumulated bloat | Population (entity tables) is a separate store from schema (defs). Recompiling the schema never rewrites population rows; nothing to carry, splice, or GC |
| Seq/Map cell duality (`ast.rs:256‑275`, `:4555‑4656`, task‑924/932) | A cell is sometimes a `Seq<fact>`, sometimes a keyed `Map<key, fact>`; merge demotes Map→Seq, the keyed shape must be re‑derived | An entity cell is a row; a junction cell is a set keyed by its spanning UC = table PK. One representation |

---

## 2. Target architecture (paper‑aligned)

**Storage unit = the entity cell `Dₙ = <Noun>:<id>`**, holding the 3NF row: RMAP‑absorbed functional facts as columns/fields. M:N, ternary+, and objectified fact types (compound/spanning UC) keep their own junction cells (`rmap.rs` Step 1).

- **In memory:** `D` is the set of entity cells (plus junction cells). `μ` folds events into the addressed entity cell (eq:cellfold). The cell substrate is the existing `Object::Map<cellName, contents>`; what changes is *which* cells hold runtime population (`Task:909`, not `Task_has_Task_Description`) and how `μ` routes into them (`EntityCellRouter`, already built).
- **On disk (SQLite host `Native` impl):** each entity cell is a **row** in its RMAP entity table (`rmap.rs` already derives `task(id, task_description, task_subject, task_status, …)` and the `task_blocks_task` junction table). Persist = upsert rows; load = read rows → entity cells. No `Object` Display/parse for values.
- **Elementary population `P`** (what `query`/`get`/`sql`/`derive`/`validate` consume) = ↑FILE reconstitution from entity cells via `reconstitute_absorbed_ft`, or a direct read of the entity table — never a separate stored blob.
- **Per target** (portability contract, `docs/11-portability.md`): the store lives behind `Native`; `apply(env, expr)` is unchanged. Cloud = DO per cell (done); local = row per entity (this rework); kernel/WASM keep in‑memory entity cells (`freeze`/`thaw` already `planned`/`stub` there).

**Schema store vs population store are separated.** Defs/metamodel are recompiled from readings each session (small, unchanged). Population is the entity tables (durable, independent of recompile). This separation is what dissolves the preserve‑dance.

---

## 3. Components and data flow

1. **`Native` entity‑cell store (new, local impl over SQLite).** The durable surface for `store`/`fetch`/`query`/`snapshot`. Schema = `rmap(state)` (`rmap.rs:320`); DDL = `generate_ddl` (`compile.rs:9138`) with CHECK re‑enabled (see §5).
2. **Persist / freeze.** For each runtime entity cell `<Noun>:<id>`, upsert one row into `to_snake(Noun)` (columns from `rmap::columns_for_table`); for each junction FT, upsert into its compound table. Routing FT→cell→table is `rmap_cell_map` + `EntityCellRouter` (both exist).
3. **Load / thaw.** `SELECT` rows from each RMAP table → build entity cells `Dₙ` (3NF rows) in the in‑memory `Object`; junction tables → junction cells. Replaces `db::load_state`'s `Object::parse` per blob (`cli/entry.rs:159‑184`, mirrored in `reload.rs`/`watch.rs`).
4. **↑FILE read view.** `query`/`get`/`sql`/`derive` read `P` through `reconstitute_absorbed_ft` over entity cells (eq:pop). The `sql` verb's `ft_*` tables (`sql.rs:127‑158`) are materialized from `P` (or directly from the persisted entity tables), eliminating the keyed‑Map mis‑read that returned `COUNT=0` today.
5. **Apply / runtime write.** `merge_delta`'s Map‑union (`ast.rs:5334‑5358`) becomes a typed upsert through `EntityCellRouter` — runtime path, retained, retargeted.

---

## 4. Migration (cut‑over, verified — non‑destructive until proven)

1. Build the entity‑cell `Native` store behind a feature flag. Live blob path untouched.
2. **One‑time migrator:** read existing FT‑keyed blob cells → route each elementary fact via `EntityCellRouter` into entity rows (+ junction rows); scrub legacy malformed rows (the one‑shot home for `drop_subjectless_facts_with_arity`'s job, §5).
3. Run on a **copy** of the good backup (`tasks.db.bak-pre-compile-gc-fix-20260528-214634`, 787 descriptions). **Full census:** `P_before` (↑FILE of old blob cells) ≡ `P_after` (↑FILE of new entity tables) — every elementary fact present, per‑FT counts equal, values byte‑identical; plus per‑entity 3NF‑row equality.
4. Flip the live board **only** when the census matches 1:1. Backups retained; no live recompile before then.

---

## 5. Behavior that must be preserved (entanglements from the deleted machinery)

- **Alethic UC enforcement** is currently lowered into `cell_put_keyed`'s `Err(KeyConflict)` (`compile.rs:7442‑7530`). It moves to the table **PRIMARY KEY / UNIQUE** (`rmap.rs` already emits these; `generate_ddl` currently **drops** CHECK at `compile.rs:9187` — **re‑enable** so VC enums are enforced).
- **φ‑canonicalization** (`canon_phi`/`same_identity`, `ast.rs:4922‑4960`): the `Atom("φ")` / empty‑Seq / `Atom("")` "no object" equivalence maps to a single typed **NULL/empty** so re‑assertion stays idempotent.
- **Deterministic key‑order output** (`fetch_cell_seq` sorting, `ast.rs:4693`) that thm:derive caching / cor:consensus replay depend on comes from `ORDER BY` the reference‑scheme PK.
- **Legacy malformed‑row scrub** (`drop_subjectless_facts_with_arity`): runs **once** in the migrator (typed columns make these unrepresentable afterward), not on every recompile.

---

## 6. Testing (live debugging + unit tests)

- **Unit per RMAP shape:** functional binary → column upsert/read; mandatory → NOT NULL; M:N → junction row; 1:1 absorption direction; compound reference scheme → composite PK; partitioned subtype → FK id.
- **Property (round‑trip):** populate → freeze → thaw → ↑FILE `== original P`. Structurally **cannot** be fooled by escaping — there is no string codec in the path.
- **Migrator census** as an integration test on a real app‑DB copy.
- **Regression for the clobber:** the 787‑description population survives freeze → thaw → schema recompile unchanged (the exact failure that motivated this work).

---

## 7. North‑star alignment ("remove Rust, add predicate readings")

- **Deletes** a large body of compensating Rust: the escape/parse codec, `_CellKeyRoles` emission + consumption, `preserve_prior_population`, `merge_states`/`concat_dedup` for population, `dedup_state_for_persist`/`dedup_cell_facts`, `drop_subjectless_facts_with_arity` (→ one migrator pass), and the Seq/Map duality branches.
- **Adds** no bespoke storage logic: the store is a mechanical projection of `rmap(state)`, which is itself derived from the readings' fact types + uniqueness constraints. Constraint enforcement moves from Rust cross‑scans to declarative table PK/UNIQUE/CHECK. Net: **less Rust, more readings‑derived structure** — and storage becomes "one function" behind `Native`, as the paper frames it.

---

## 8. Risks / open questions (concrete)

1. **In‑memory read‑path blast radius.** Derivations/queries that today read FT cells (`Task_has_Task_Description`) must read `P` via ↑FILE (or read entity cells). The `Object::Map<cellName, …>` substrate and `fetch`/`store` are generic, and `EntityCellRouter` + `reconstitute_absorbed_ft` already bridge — but the alethic‑UC enforcement read path (`command.rs:1021/2015/2544`) needs care. **First implementation spike:** enumerate every consumer that assumes an FT‑keyed cell and confirm each has an ↑FILE/entity‑cell equivalent.
2. **Junction‑cell keying for M:N with no UC** (`rmap_cell_map` "No UC → own cell"): confirm the synthetic key is stable across freeze/thaw.
3. **no_std/kernel freeze is `planned`** (portability table): this rework targets the local SQLite `Native` first; kernel/WASM keep in‑memory entity cells, no on‑disk format change required by this work.
4. **Phasing.** Large enough to land in stages: (a) entity‑cell store + migrator + census behind a flag; (b) flip load/persist; (c) retire the dead machinery once green. Each stage independently testable.

---

## 9. Blast radius & revised risk (consumer audit, 2026‑05‑29)

A read‑only audit of every population‑cell consumer (compiled evaluator + apply + derivation + induce + REST/MCP surfaces) refines §8.1 and the §5 UC story. Headline: **the read path is largely already ↑FILE‑aware; the write path and UC enforcement are the real work.**

**Read path — mostly already done.** The compiled evaluator reads population through `Func::Fetch`/`FetchOrPhi` (`ast.rs:2409/2454`), which already fall through to `resolve_view → reconstitute_absorbed_ft`. So derivation antecedents (`extract_facts_from_pop`), the MCP `query` verb (`query.rs:26`), `platform_query_ft`, and `sql.rs` materialization read absorbed FTs correctly with **no change**. The remaining reads are *raw helper* reads (`fetch_cell_seq` / `cells_iter`+`cell_facts_iter`) that bypass ↑FILE — mechanical redirects.

**Write path — `EntityCellRouter` must become primary.** Today the router (`lib.rs:2383 augment_delta_with_entity_cells`) *derives* `<Noun>:<id>` cells **alongside** the FT‑cell writes; the real writers still key by FT id: `evaluate.rs:90 integrate_round_facts` (forward‑chain fold), `command.rs push_with_uc_check` + the create/update/transition/SM write sites, and `induce.rs` candidate injection — all via `cell_put_keyed`/`cell_put_folded`/`cell_push`. These must route through the router so entity cells are the store, not a derived shadow.

**UC enforcement — load‑bearing, but smaller than it first looks.** Alethic UC is enforced *inside* `cell_put_keyed` (`KeyConflict` on a same‑reference‑scheme‑key write), read at `command.rs:1021/2015/2544` + `induce.rs:322` + the `command.rs:958` pre‑check, shared by the chain at `evaluate.rs:114`. In the entity‑cell model it **splits by UC kind**:
- **Functional UCs** (absorb as columns) become **structural** — the entity cell holds a single‑valued field; a second write last‑writes. The majority of UCs; *simpler*.
- **Reference‑scheme uniqueness** becomes entity‑cell‑namespace uniqueness: "does `Task:909` already exist?" (replaces the `command.rs:958` raw scan).
- **Non‑functional UCs** (1:1 *reverse*, external/spanning cross‑FT, junction spanning) need an explicit cross‑entity check → persisted **SQL `UNIQUE`** (rmap already emits these) + an **in‑memory uniqueness index** built per snapshot. This residual is the net‑new code; `EntityCellRouter` has no `KeyConflict` counterpart yet.

**Genuine gaps needing new code (4):**
1. `encode_state`/`encode_state_indexed` (`ast.rs:651/618`) build the validate/derive population from raw `cells_iter` without reconstitution — the `Selector(3)` Seq form reads absorbed FTs empty.
2. `evaluate.rs:594 state_keys` (chain dedup/fixpoint index) keys off raw cell contents — must key off the entity‑cell shape or re‑fire loops appear.
3. `platform_list_noun` / `visible_population` / `hateoas.rs` actively **exclude `:`‑named cells** — exactly the `<Noun>:<id>` names the rework introduces; needs redesign.
4. Proof engine `evaluate.rs:672/701` axiom‑searches raw cells.

**Revised phasing (supersedes §8.4):**
- **P0 — UC‑enforcement spike (de‑risk first).** Prototype the in‑memory entity‑cell uniqueness index + the structural‑UC collapse on one entity (`Task`) and prove alethic rejection still fires. The load‑bearing unknown; settle before committing. **✅ P0 SETTLED (commit `b6be4ded`):** verified in `crates/arest/src/entity_uc.rs` — 6 tests green. Functional UCs are structural (single‑valued entity‑cell field); reference‑scheme uniqueness is a namespace lookup (`entity_exists`); non‑functional UCs (1:1‑reverse / external / junction) use `EntityUniquenessIndex`, raising the same `uc:{name}` / `alethic:true` `Violation` the apply path rejects on. No dead end — **P1 unblocked.**
- **P1 — entity‑cell store + migrator + census** behind a flag (read path leans on existing ↑FILE).
- **P2 — re‑home writers** through `EntityCellRouter`; fix the 4 gaps; redirect raw helper reads.
- **P3 — flip load/persist; retire** the FT‑blob store + preserve/dedup/keyroles machinery once census + property tests are green.
