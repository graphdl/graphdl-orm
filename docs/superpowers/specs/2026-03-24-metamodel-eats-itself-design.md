# Metamodel Eats Its Own Tail

Collapse DomainDB into EntityDB DOs so the metamodel is just another domain. Every entity — whether a Noun definition, a Reading definition, or a Customer instance — is an EntityDB DO indexed by RegistryDB. The Payload CMS abstraction layer is deleted. Ingestion follows Halpin's CSDP and RMAP by the book, with inductive constraint discovery at the claims level.

## Core Principle

The readings in `core.md`, `organizations.md`, `state.md`, `instances.md`, `agents.md`, `ui.md`, and `validation.md` define the metamodel. Seeding the "core" domain produces EntityDB DOs for each metamodel entity — the same way seeding a "tickets" domain produces EntityDB DOs for SupportRequest and Priority. `ui.md` defines presentation-layer entities (Dashboard, Widget, Entity List). `validation.md` defines meta-constraints (deontic rules about the modeling discipline itself).

- **Entity-typed nouns** (Object Type = 'entity') — instances are EntityDB DOs, indexed in RegistryDB
- **Value-typed nouns** (Object Type = 'value') — instances are fields within entity data blobs, not separate DOs

The metamodel entities themselves (Noun, Reading, Constraint definitions) are always EntityDB DOs — because `Noun` is an entity type in the metamodel domain.

## Architecture: Three DOs

### EntityDB (unchanged)

One DO per entity instance. Holds:
- Single `entity` row (id, type, JSON data blob, version, timestamps, soft-delete)
- CDC `events` log (create/update/delete with prev snapshots)
- CRUD via `get/put/patch/delete/events` methods

### RegistryDB (updated)

One DO per scope (org/public). Holds:
- `domains` table: slug to DO ID mapping with visibility
- `noun_index`: which domain owns which noun type
- `entity_index`: noun type to entity ID listing (with soft-delete) — **add `domain_slug` column** for efficient per-domain queries without full fan-out
- Resolution via `resolveNounInChain` across [org, public] registries (reduced from three-tier [app, org, global])

**Scope simplification:** Remove app-level domains. Two scopes only — **org** and **public**. Apps select/lasso which org domains they use; they don't own domains. This requires:
- Update `core.md` Scope value type from `'local', 'app', 'organization', 'public'` to `'organization', 'public'`
- Update `resolveNounInChain` from three-registry to two-registry chain
- Migrate any existing app-scoped RegistryDB DOs: re-index their entities under the parent org scope

### DomainDB (reduced to batch WAL)

One DO per domain. Two tables:

**`batches`** — transactional integrity for ingestion:
- `id` — batch UUID
- `status` — pending / committed / failed
- `entities` — JSON array of `{ id, type, domain, data }` entries
- `created_at` — timestamp

**`generators`** — cached generation output (implementation artifact, not a metamodel entity):
- `id`, `domain_id`, `output_format`, `title`, `version`, `output`

A batch is always scoped to a single domain (one DomainDB DO). Multi-domain ingestion is orchestrated by the apis worker, which posts separate batches per domain.

Purpose: transactional integrity for ingestion. The batch is the unit of atomicity.

Flow:
1. Receive ingestion request (parsed claims after CSDP validation)
2. Write entire batch as one row — single SQLite transaction, atomic
3. Fan out to EntityDB DOs (create each entity)
4. Update RegistryDB indexes (including `domain_slug` on `entity_index`)
5. Mark batch as committed

If fan-out fails partway, the batch row has the complete intended state. Retry picks up where it left off. Readers only see entities that made it to their EntityDB DOs. Incomplete batches get retried until complete.

Also serves: ingestion idempotency (has this reading been ingested?), domain-level operations (wipe/re-seed via batch log), audit (when was this reading ingested, what did it produce?).

**CDC:** Domain-level CDC aggregation moves to the Worker layer. Each EntityDB DO has its own CDC events table. Domain-wide CDC feeds are assembled by querying entity events via fan-out, or by the batch WAL logging which entities were created/updated per batch. WebSocket broadcast uses the batch commit as the trigger, not individual entity writes.

## Write Path: Progressive Induction + CSDP + RMAP

Induction runs three times during ingestion, each time over a larger population with higher confidence. Induction is cheap (in-memory population analysis via WASM, no LLM calls). Each round feeds its discovered constraints into the next stage as context.

### Progressive Induction

**Round 1: After deterministic parse (apis layer)**

```
deterministic parse -> population_1 (instance facts from FORML2 text)
  induce(population_1) -> constraints_1 (low confidence, small population)
```

Discovers initial UC/MC/FC/SS patterns from whatever instance facts the parser found. Low confidence (typically 2-5 instances per fact type), but enough to seed the LLM with discovered patterns. If the deterministic parse has high coverage and induction finds strong constraints, the LLM may not be needed at all.

**Round 2: After LLM extraction (apis layer)**

```
LLM extraction (with constraints_1 as context) -> new claims
merge deterministic + LLM claims -> population_2
  induce(population_2) -> constraints_2 (medium confidence, larger population)
```

The LLM receives constraints_1 as context — it knows what the system already discovered and can validate, refine, or add to them rather than inventing from scratch. After merging, induction runs again on the combined population. Confidence scores climb as population grows. A UC induced at 0.6 from 2 facts after parse might reach 0.85 after the LLM adds 5 more instances.

**Round 3: During CSDP validation (graphdl-orm /claims)**

```
CSDP Steps 1-3 + forward-chain derivation rules -> population_3
  induce(population_3) -> constraints_3 (high confidence, full picture)
```

The authoritative pass. Runs after forward-chaining has produced derived facts, so the population includes both base and derived facts. This is where induced constraints reach their highest confidence and feed into the validation gate.

### CSDP (claims to validated conceptual schema)

The claims ingestion pipeline implements Halpin's seven CSDP steps. Invalid schemas are rejected with proposed fixes.

**Step 1: Transform examples into elementary facts, quality checks.**
`parseFORML2()` verbalizes text into elementary facts. Elementarity check (and-test, arity). The apis worker's hybrid pipeline handles the LLM integration — deterministic parse first, LLM extracts residue only if needed. Induction rounds 1-2 run during this stage at the apis layer.

**Step 2: Draw fact types, population check.**
Build in-memory schema from ExtractedClaims (including constraints from induction rounds 1-2). Validate sample instance facts against fact types.

**Step 3: Combine entity types, note derivations.**
Deduplicate nouns across claims (same name + same reference mode = same entity). Mark arithmetic derivations (`:=` rules). Forward-chain derivation rules to expand the population.

**Step 4: Add uniqueness constraints, check arity.**
Induction round 3 runs here on the full population (base + derived facts). Discovers UCs with high confidence. Arity check: if a ternary has a UC spanning fewer than n-1 roles, reject and propose splitting into binaries.

**Step 5: Add mandatory role constraints, check logical derivations.**
Induction round 3 discovers MCs (every instance plays the role). Check for logical derivations not already noted.

**Step 6: Add value, set-comparison, subtyping constraints.**
Value constraints from enum declarations. SS induction from population subset analysis. Subtype constraints from `is subtype of` declarations — validate totality/exclusion.

**Step 7: Other constraints, final checks.**
Ring constraints on self-referencing facts. Frequency constraints. Validation gate: completeness, redundancy, consistency, population check.

### Validation Gate

If the schema is invalid, the batch is **rejected** (not committed to the WAL). The response includes violations and proposed fixes:

- **Arity violation** (UC spanning < n-1 on ternary) — propose binary decomposition
- **Missing mandatory constraint** (induction found MC but declaration is missing) — propose adding it
- **Conflicting constraints** — identify the contradiction, propose which to drop
- **Undeclared noun** in a constraint — propose entity/value type declaration
- **Non-elementary fact** (and-test fails) — propose split into separate readings
- **Missing subtype constraint** — subtypes declared without totality/exclusion
- **Ring constraint missing** — self-referential binary without ring constraint

Induced constraints with high confidence (>= 0.8) that were not explicitly declared are included in the validation response as **proposed constraints** — the user or LLM can accept or reject them before resubmitting.

### RMAP (validated schema to relational artifacts)

After CSDP validation passes, RMAP generates the relational schema from the in-memory validated model per Halpin Chapter 10:

- **Step 0.1:** Binarize unaries (exclusive unaries to status code binary)
- **Step 0.2:** Erase reference predicates, treat composites as black boxes
- **Step 0.3:** Subtype absorption overrides (default: absorb into supertype)
- **Step 0.4:** Mark stored derivations
- **Step 0.5:** Symmetric 1:1 choices (favor fewer nulls)
- **Step 0.6:** Disjunctive reference schemes (artificial ID or concatenation)
- **Step 0.7:** Objectified associations without spanning UC
- **Step 1:** Compound UC to separate table (M:N binaries, ternaries with compound UC)
- **Step 2:** Functional roles grouped by entity, keyed on identifier
- **Step 3:** 1:1 absorbed, favor fewer nulls
- **Step 4:** Independent entity to single-column table
- **Step 5:** Unpack black boxes (composite identifiers expanded)
- **Step 6:** Map constraints (UCs to keys, MCs to NOT NULL, SS to FK, value to CHECK, ring to CHECK/trigger)

RMAP output feeds the existing generators (`generateSQLite`, `generateOpenAPI`, etc.) but now driven by the formal procedure instead of ad-hoc mapping tables.

### Complete Ingestion Flow

```
Natural language text
  -> apis: deterministic parse -> population_1
  -> apis: induce(population_1) -> constraints_1 (round 1)
  -> apis: LLM hybrid extraction (constraints_1 as context) -> new claims
  -> apis: merge claims -> population_2
  -> apis: induce(population_2) -> constraints_2 (round 2)
  -> graphdl-orm /claims (with constraints_2):
      CSDP Steps 1-3 (build schema, deduplicate, forward-chain)
      CSDP Step 4-6 induce(population_3) -> constraints_3 (round 3)
      CSDP Step 7 (validate -- REJECT with proposed fixes if invalid)
      -> if valid:
         RMAP (generate relational artifacts from in-memory validated schema)
         DomainDB.commitBatch() (atomic WAL write)
         materialize to EntityDB DOs + RegistryDB indexes
         evaluate all constraints against population
```

**Prerequisite:** The WASM `induce_from_population` function must be callable from both the apis worker (rounds 1-2) and graphdl-orm (round 3). The Rust FOL engine already implements UC/MC/FC/SS/derivation rule induction. The TypeScript binding needs to be available in both workers — either via a shared WASM module or by apis calling graphdl-orm's `/induce` endpoint for rounds 1-2.

## Read Path

### Entity Queries

Replace Payload-style WHERE clause queries with Registry fan-out:

```
GET /api/entities/Noun?domain=tickets
  -> RegistryDB.getEntityIds('Noun', domain='tickets') -> [id1, id2, ...]
  -> fan-out: EntityDB(id1).get(), EntityDB(id2).get(), ...
  -> return JSON array
```

The `domain_slug` column on `entity_index` enables efficient per-domain queries without fetching all entities of a type across all domains.

For field-level filtering (currently `where[text][like]=...`): the Worker filters in-memory after fan-out. Entities are JSON blobs — filtering on fields is a `.filter()` over the collected results. This is bounded by the entity count within the requested domain+type, not across all domains.

For read-path fan-out failures: if an individual EntityDB DO is unreachable, the result omits that entity and includes a `warnings` array noting the missing IDs. The caller decides whether to retry.

### Depth Population

Entity data blobs contain IDs referencing other entities. A second fan-out resolves those IDs to their entity data. Replaces FK_TARGET_TABLE / REVERSE_FK_MAP machinery.

### DomainModel Loader

`SqlDataLoader` (queries DomainDB SQL tables) replaced by `EntityDataLoader` (fans out to EntityDB DOs by type via Registry). DomainModel's public interface unchanged — `nouns()`, `factTypes()`, `constraints()`, etc. Generators consume DomainModel as before.

Multi-entity joins (e.g., constraints -> spans -> roles) require multi-step fan-outs. The EntityDataLoader fetches entities by type in parallel batches, then resolves cross-references in memory. This trades SQL joins for DO fan-out + in-memory linking — acceptable because the entity counts per domain are bounded (typically tens to low hundreds of metamodel entities).

### RMAP Execution

RMAP runs from the in-memory validated schema (the output of CSDP Step 7), not by reading back from EntityDB DOs. The validated schema is already fully materialized in memory at the point of CSDP validation. This avoids an unnecessary write-then-read round-trip. RMAP output (generated artifacts) is cached in the DomainDB `generators` table.

## Cascade Deletes

Deleting a metamodel entity (e.g., a Noun) requires cascading to dependent entities (readings, roles, constraints referencing that noun). In the current DomainDB, this is handled by SQLite foreign keys and the CASCADE_MAP in `domain-do.ts`.

In the new architecture, cascade deletes are handled by the batch WAL: a delete batch lists the root entity and all its dependents (discovered by querying the Registry for entities that reference the root). The batch is committed atomically — all entities are soft-deleted in their EntityDB DOs and de-indexed in RegistryDB. If fan-out fails partway, the batch retries.

The dependency graph for cascades is derived from the metamodel itself: the readings in `core.md` declare which entity types reference which others (e.g., "Graph Schema has Reading" means deleting a Graph Schema cascades to its Readings).

## What Gets Deleted

- **`src/domain-do.ts`** query engine (~500 lines) — findInMetamodel, createInMetamodel, buildWhereClause, updateInMetamodel, deleteFromMetamodel, FK traversal, field mapping. Replaced by batch WAL (one table).
- **`src/collections.ts`** — COLLECTION_TABLE_MAP (45+ entries), FIELD_MAP (1000+ translations), FK_TARGET_TABLE, REVERSE_FK_MAP, NOUN_TABLE_MAP. All Payload legacy.
- **`src/do-adapter.ts`** — GraphDLDBLike bridge interface. Gone.
- **`src/api/collections.ts`** — parsePayloadWhereParams, Payload URL query syntax. Gone.
- **`src/schema/bootstrap.ts`** — 20+ hardcoded DDL statements. Generated from readings instead.
- **`src/wipe-tables.ts`** — no tables to wipe.

## What Gets Rewritten

- **`src/model/domain-model.ts`** — SqlDataLoader replaced by EntityDataLoader (fan-out to EntityDB DOs by type via Registry). DomainModel public interface unchanged.
- **`src/hooks/`** — currently call `db.createInCollection()`. New: add entities to the in-memory batch during ingestion. Same parse logic, different write target.
- **`src/api/router.ts`** — replace resolveDomainDB() / getPrimaryDB() with Registry-based routing. Collection CRUD endpoints become entity-type endpoints.
- **`src/claims/steps.ts`** — currently calls `db.createInCollection()` / `db.findInCollection()`. New: builds batch entities, uses scope for deduplication. Add CSDP validation gate + induction calls.
- **`src/domain-do.ts`** — rewritten from 500+ line query engine to ~50 line batch WAL (single `batches` table, commitBatch method, fan-out materializer).

## What Stays the Same

- **EntityDB DO** — structure unchanged (id, type, JSON data, version, CDC events)
- **RegistryDB** — unchanged (domain registration, noun indexing, entity indexing)
- **Parsers** — `parseFORML2()`, `parseOrmXml()` output ExtractedClaims
- **FOL engine** (Rust/WASM) — `load_ir`, `evaluate_response`, `synthesize_noun`, `forward_chain`, `query_population`, `induce_from_population`, `prove_goal`
- **Generators** — consume DomainModel, produce OpenAPI/SQLite/XState/MDXUI/readings/readme. Only the data loader changes.
- **apis worker** — calls graphdl-orm via service binding. HTTP contract unchanged for `/parse`, `/claims`, `/api/entity`, `/api/evaluate`. Query endpoints get new URL patterns but same JSON response shape.

## APIs Integration

The apis worker is the orchestration layer. Its pipeline:

1. Deterministic parse first (graphdl-orm `/parse`) — handles well-formed FORML2 with no LLM cost
2. Quality check (parse ratio, coverage threshold)
3. If partial coverage: LLM extracts residue only (hybrid mode, coded shortcodes mark parsed elements)
4. If no coverage: full LLM extraction
5. Merge deterministic + semantic claims, deduplicate
6. POST to graphdl-orm `/claims` — CSDP validates, rejects if invalid

The apis layer does not need significant changes. It calls graphdl-orm via service binding (`env.GRAPHDL.fetch()`). The internal storage change (DomainDB tables to EntityDB DOs) is invisible to apis. Query endpoints get new URL patterns but the apis helpers (`graphdlFind`, `graphdlPost`, etc.) adapt to the new patterns.

Agent-chat loads readings + deontic constraints from the domain, gives them to the LLM as system context. Constraint evaluation runs the WASM FOL engine on responses, redrafts up to 3x if violations found. This flow is unchanged.

## Migration

1. Seed the core domain readings (`core.md`, `organizations.md`, `state.md`, `instances.md`, `agents.md`) through the new CSDP pipeline first — they define the metamodel entity types as EntityDB DOs.
2. Existing domain data is migrated by re-ingesting each domain's readings through the new pipeline.
3. wrangler.jsonc migration: add `v4` tag that drops DomainDB's metamodel tables (keep only the `batches` table).
4. Delete Payload layer code after all reads go through EntityDB DOs.

## Reading Gaps Closed

As part of this design, the following readings were added to `core.md` to close DDL-reading gaps:

- `World Assumption` value type ('closed', 'open') + `Noun has World Assumption`
- `Noun is independent` (unary)
- `Graph Schema is derived` (unary)
- `Reading is primary` (unary)
- `Constraint has Text` (optional)
- `Role has Position for Reading` (ternary UC)

The `generators` table is an implementation artifact (cached generation output), not a domain concept. It remains in DomainDB as a cache alongside the `batches` table.

## Halpin Source References

- CSDP: Halpin, *Information Modeling and Relational Databases*, Chapters 3-7 (pp. 59-334)
- RMAP: Halpin, *Information Modeling and Relational Databases*, Chapter 10 (pp. 403-456)
- Verbalization: Halpin, Curland & CS445, "ORM 2 Constraint Verbalization Part 1" (TechReport ORM2-02, 2006)
- Automated Verbalization: Halpin & Curland, "Automated Verbalization for ORM 2" (CAiSE'06 Workshops)
- Objectification: Halpin, "Objectification and Atomicity" (2020)
- Induction: FOL engine `induce.rs` implements UC/MC/FC/SS/derivation rule discovery per CSDP Step 4
