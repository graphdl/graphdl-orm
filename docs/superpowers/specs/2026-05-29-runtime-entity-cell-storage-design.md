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
