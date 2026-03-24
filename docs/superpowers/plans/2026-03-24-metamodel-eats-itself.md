# Metamodel Eats Its Own Tail — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse DomainDB into EntityDB DOs so the metamodel is just another domain, with progressive induction and formal CSDP/RMAP pipeline.

**Architecture:** Every entity (metamodel definitions and user-domain instances) is an EntityDB DO indexed by RegistryDB. DomainDB becomes a batch WAL for transactional integrity. Ingestion follows Halpin's CSDP with three rounds of inductive constraint discovery. The Payload CMS abstraction layer is deleted entirely.

**Tech Stack:** TypeScript (Cloudflare Workers), Rust/WASM (FOL engine), vitest, wrangler

**Spec:** `docs/superpowers/specs/2026-03-24-metamodel-eats-itself-design.md`

---

## Phase 1: Foundation (RegistryDB + Batch WAL)

### Task 1: Add `domain_slug` to RegistryDB `entity_index`

**Files:**
- Modify: `src/registry-do.ts`
- Modify: `src/registry-do.test.ts`

- [ ] **Step 1: Write failing test for domain-scoped entity indexing**

```typescript
// in registry-do.test.ts
describe('domain-scoped entity_index', () => {
  it('indexes entities with domain_slug', () => {
    initRegistrySchema(sql)
    indexEntity(sql, 'Noun', 'entity-1', 'tickets')
    indexEntity(sql, 'Noun', 'entity-2', 'billing')
    const ticketNouns = getEntityIds(sql, 'Noun', 'tickets')
    expect(ticketNouns).toEqual(['entity-1'])
  })

  it('returns all entities when no domain filter', () => {
    initRegistrySchema(sql)
    indexEntity(sql, 'Noun', 'entity-1', 'tickets')
    indexEntity(sql, 'Noun', 'entity-2', 'billing')
    const allNouns = getEntityIds(sql, 'Noun')
    expect(allNouns).toEqual(['entity-1', 'entity-2'])
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/registry-do.test.ts -t "domain-scoped"`
Expected: FAIL — `indexEntity` doesn't accept domain_slug parameter

- [ ] **Step 3: Update schema and functions**

Add `domain_slug` column to `entity_index` table:

```typescript
sql.exec(`CREATE TABLE IF NOT EXISTS entity_index (
  noun_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  domain_slug TEXT,
  deleted INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (noun_type, entity_id)
)`)
```

Update `indexEntity` signature:
```typescript
export function indexEntity(sql: SqlLike, nounType: string, entityId: string, domainSlug?: string): void {
  sql.exec(
    `INSERT OR REPLACE INTO entity_index (noun_type, entity_id, domain_slug, deleted) VALUES (?, ?, ?, ?)`,
    nounType, entityId, domainSlug || null, 0,
  )
}
```

Update `getEntityIds` to accept optional domain filter:
```typescript
export function getEntityIds(sql: SqlLike, nounType: string, domainSlug?: string): string[] {
  const rows = domainSlug
    ? sql.exec(`SELECT entity_id FROM entity_index WHERE noun_type=? AND domain_slug=? AND deleted=0`, nounType, domainSlug).toArray()
    : sql.exec(`SELECT entity_id FROM entity_index WHERE noun_type=? AND deleted=0`, nounType).toArray()
  return rows.map((row: any) => row.entity_id)
}
```

Update `RegistryDB` class methods to pass through `domainSlug`.

Add migration for existing rows: `try { sql.exec('ALTER TABLE entity_index ADD COLUMN domain_slug TEXT') } catch { /* already exists */ }`

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/registry-do.test.ts`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/registry-do.ts src/registry-do.test.ts
git commit -m "feat(registry): add domain_slug to entity_index for per-domain queries"
```

---

### Task 2: Reduce scope from three-tier to two-tier

**Files:**
- Modify: `src/resolution.ts`
- Modify: `src/resolution.test.ts`
- Modify: `readings/core.md`

- [ ] **Step 1: Update `readings/core.md` Scope value type**

Change from:
```
Scope is a value type.
  The possible values of Scope are 'local', 'app', 'organization', 'public'.
```
To:
```
Scope is a value type.
  The possible values of Scope are 'organization', 'public'.
```

This must happen before Task 21 (core domain seed) or the seeded metamodel will have the wrong Scope definition.

- [ ] **Step 2: Write tests for two-tier resolution**

```typescript
describe('two-tier resolution (org -> public)', () => {
  it('resolves noun in org registry first', async () => {
    const orgRegistry = { resolveNoun: vi.fn().mockResolvedValue({ domainSlug: 'tickets', domainDoId: 'do-1' }) }
    const publicRegistry = { resolveNoun: vi.fn().mockResolvedValue(null) }
    const result = await resolveNounInChain('Customer', [orgRegistry, publicRegistry])
    expect(result).toEqual({ domainSlug: 'tickets', domainDoId: 'do-1', registryIndex: 0 })
    expect(publicRegistry.resolveNoun).not.toHaveBeenCalled()
  })

  it('falls through to public if org has no match', async () => {
    const orgRegistry = { resolveNoun: vi.fn().mockResolvedValue(null) }
    const publicRegistry = { resolveNoun: vi.fn().mockResolvedValue({ domainSlug: 'core', domainDoId: 'do-2' }) }
    const result = await resolveNounInChain('Noun', [orgRegistry, publicRegistry])
    expect(result).toEqual({ domainSlug: 'core', domainDoId: 'do-2', registryIndex: 1 })
  })
})
```

- [ ] **Step 3: Run test to verify it passes**

Run: `npx vitest run src/resolution.test.ts`
Expected: PASS — `resolveNounInChain` is already generic over any number of registries. The two-tier change is in the callers (router rewrite, Task 16-17).

- [ ] **Step 4: Update resolution.ts comment**

Change line 8 from `"ordered by priority: [app, org, global]"` to `"ordered by priority: [org, public]"`.

- [ ] **Step 5: Commit**

```bash
git add src/resolution.ts src/resolution.test.ts readings/core.md
git commit -m "feat(resolution): reduce scope to two-tier (org, public), update core.md Scope values"
```

---

### Task 3: Build DomainDB batch WAL

**Files:**
- Create: `src/batch-wal.ts`
- Create: `src/batch-wal.test.ts`

- [ ] **Step 1: Write failing test for batch WAL**

```typescript
// batch-wal.test.ts
import { describe, it, expect, beforeEach } from 'vitest'
import { initBatchSchema, createBatch, getBatch, markCommitted, markFailed, getPendingBatches } from './batch-wal'

function mockSql() {
  const db = new Map<string, any[]>()
  return {
    exec(query: string, ...params: any[]) {
      // in-memory SQLite mock
      // ... (same pattern as existing tests)
    }
  }
}

describe('batch WAL', () => {
  it('creates a batch with pending status', () => {
    const sql = mockSql()
    initBatchSchema(sql)
    const batch = createBatch(sql, 'tickets', [
      { id: 'n1', type: 'Noun', domain: 'tickets', data: { name: 'Customer', objectType: 'entity' } },
      { id: 'r1', type: 'Reading', domain: 'tickets', data: { text: 'Customer has Name' } },
    ])
    expect(batch.id).toBeDefined()
    expect(batch.status).toBe('pending')
    expect(batch.entityCount).toBe(2)
  })

  it('marks batch as committed', () => {
    const sql = mockSql()
    initBatchSchema(sql)
    const batch = createBatch(sql, 'tickets', [{ id: 'n1', type: 'Noun', domain: 'tickets', data: {} }])
    markCommitted(sql, batch.id)
    const fetched = getBatch(sql, batch.id)
    expect(fetched?.status).toBe('committed')
  })

  it('returns pending batches for retry', () => {
    const sql = mockSql()
    initBatchSchema(sql)
    createBatch(sql, 'tickets', [{ id: 'n1', type: 'Noun', domain: 'tickets', data: {} }])
    createBatch(sql, 'tickets', [{ id: 'n2', type: 'Noun', domain: 'tickets', data: {} }])
    markCommitted(sql, 'first-batch-id') // won't match
    const pending = getPendingBatches(sql)
    expect(pending.length).toBe(2)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/batch-wal.test.ts`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement batch WAL pure functions**

```typescript
// batch-wal.ts
export interface SqlLike {
  exec(query: string, ...params: any[]): { toArray(): any[] }
}

export interface BatchEntity {
  id: string
  type: string
  domain: string
  data: Record<string, unknown>
}

export interface Batch {
  id: string
  domain: string
  status: 'pending' | 'committed' | 'failed'
  entities: BatchEntity[]
  entityCount: number
  createdAt: string
}

export function initBatchSchema(sql: SqlLike): void {
  sql.exec(`CREATE TABLE IF NOT EXISTS batches (
    id TEXT PRIMARY KEY,
    domain TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'committed', 'failed')),
    entities TEXT NOT NULL,
    entity_count INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    committed_at TEXT,
    error TEXT
  )`)
}

export function createBatch(sql: SqlLike, domain: string, entities: BatchEntity[]): Batch {
  const id = crypto.randomUUID()
  const now = new Date().toISOString()
  const entitiesJson = JSON.stringify(entities)
  sql.exec(
    `INSERT INTO batches (id, domain, status, entities, entity_count, created_at) VALUES (?, ?, ?, ?, ?, ?)`,
    id, domain, 'pending', entitiesJson, entities.length, now,
  )
  return { id, domain, status: 'pending', entities, entityCount: entities.length, createdAt: now }
}

export function getBatch(sql: SqlLike, id: string): Batch | null {
  const rows = sql.exec(`SELECT * FROM batches WHERE id = ?`, id).toArray()
  if (rows.length === 0) return null
  const row = rows[0] as any
  return {
    id: row.id,
    domain: row.domain,
    status: row.status,
    entities: JSON.parse(row.entities),
    entityCount: row.entity_count,
    createdAt: row.created_at,
  }
}

export function markCommitted(sql: SqlLike, id: string): void {
  const now = new Date().toISOString()
  sql.exec(`UPDATE batches SET status = 'committed', committed_at = ? WHERE id = ?`, now, id)
}

export function markFailed(sql: SqlLike, id: string, error: string): void {
  sql.exec(`UPDATE batches SET status = 'failed', error = ? WHERE id = ?`, error, id)
}

export function getPendingBatches(sql: SqlLike): Batch[] {
  const rows = sql.exec(`SELECT * FROM batches WHERE status = 'pending' ORDER BY created_at ASC`).toArray()
  return rows.map((row: any) => ({
    id: row.id,
    domain: row.domain,
    status: row.status as 'pending',
    entities: JSON.parse(row.entities),
    entityCount: row.entity_count,
    createdAt: row.created_at,
  }))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/batch-wal.test.ts`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/batch-wal.ts src/batch-wal.test.ts
git commit -m "feat: add batch WAL pure functions for transactional ingestion"
```

---

### Task 4: Wire batch WAL into DomainDB DO

**Files:**
- Modify: `src/domain-do.ts`
- Create: `src/batch-do.test.ts`

- [ ] **Step 1: Write failing test for DomainDB.commitBatch**

```typescript
// batch-do.test.ts
describe('DomainDB.commitBatch', () => {
  it('writes batch to WAL and returns batch id', () => {
    // Test that commitBatch calls createBatch on the internal SQL
    // and returns the batch metadata
  })
})
```

- [ ] **Step 2: Add `commitBatch` method to DomainDB class**

Import `initBatchSchema` and `createBatch` from `batch-wal.ts`. Call `initBatchSchema` in `ensureInit`. Add `commitBatch(entities: BatchEntity[]): Promise<Batch>` method.

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/batch-do.test.ts`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/domain-do.ts src/batch-do.test.ts
git commit -m "feat(domain-do): add commitBatch method backed by batch WAL"
```

---

## Phase 2: Entity Data Loader

### Task 5: Create EntityDataLoader

**Files:**
- Create: `src/model/entity-data-loader.ts`
- Create: `src/model/entity-data-loader.test.ts`

- [ ] **Step 1: Write failing test**

```typescript
// entity-data-loader.test.ts
describe('EntityDataLoader', () => {
  it('loads nouns by fan-out to entity stubs', async () => {
    const entities = [
      { id: 'n1', type: 'Noun', data: { name: 'Customer', objectType: 'entity', domainId: 'd1' } },
      { id: 'n2', type: 'Noun', data: { name: 'Order', objectType: 'entity', domainId: 'd1' } },
    ]
    const stubs = new Map(entities.map(e => [e.id, { get: vi.fn().mockResolvedValue(e) }]))
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['n1', 'n2']) }
    const loader = new EntityDataLoader(registry, (id) => stubs.get(id)!)
    const nouns = await loader.queryNouns('d1')
    expect(nouns).toHaveLength(2)
    expect(nouns[0].name).toBe('Customer')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/model/entity-data-loader.test.ts`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement EntityDataLoader**

Implements the `DataLoader` interface from `src/model/domain-model.ts`. Each method:
1. Calls `registry.getEntityIds(type, domainSlug)` to get entity IDs
2. Fans out to EntityDB stubs in parallel batches of 50
3. Filters and transforms results to match the expected return types
4. For cross-references (e.g., constraint -> spans -> roles), does multi-step fan-out with in-memory linking

- [ ] **Step 4: Run tests**

Run: `npx vitest run src/model/entity-data-loader.test.ts`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/model/entity-data-loader.ts src/model/entity-data-loader.test.ts
git commit -m "feat(model): add EntityDataLoader with Registry fan-out"
```

---

### Task 6: Add tests for multi-entity joins in EntityDataLoader

**Files:**
- Modify: `src/model/entity-data-loader.test.ts`

- [ ] **Step 1: Write failing tests for constraint -> span -> role resolution**

Test that `queryConstraintSpans(domainId)` fetches Constraint Span entities, then resolves their referenced Role and Constraint IDs via secondary fan-outs.

- [ ] **Step 2: Implement the join logic in EntityDataLoader**

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/model/entity-data-loader.test.ts`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/model/entity-data-loader.ts src/model/entity-data-loader.test.ts
git commit -m "feat(model): add multi-entity join resolution to EntityDataLoader"
```

---

### Task 6b: Make DataLoader interface async

**Files:**
- Modify: `src/model/domain-model.ts`
- Modify: `src/model/domain-model.test.ts`

The existing `DataLoader` interface returns synchronous `Row[]` from every method. `EntityDataLoader` does async fan-out, so every method must return `Promise<Row[]>`. This cascades through `DomainModel` (which must `await` every loader call) and into every generator that calls `DomainModel`.

- [ ] **Step 1: Change `DataLoader` interface methods to return `Promise<Row[]>`**

Update every method signature in the `DataLoader` interface (lines 35-49 of `domain-model.ts`).

- [ ] **Step 2: Update `SqlDataLoader` to return `Promise.resolve(rows)`**

Wrap existing synchronous returns in `Promise.resolve()` for backward compatibility during migration.

- [ ] **Step 3: Update `DomainModel` to await all loader calls**

Every call like `const rows = this.loader.queryNouns(this.domainId)` becomes `const rows = await this.loader.queryNouns(this.domainId)`. All DomainModel accessor methods (`nouns()`, `factTypes()`, `constraints()`, etc.) become async.

- [ ] **Step 4: Update all generator call sites to await DomainModel methods**

Generators that call `model.nouns()` must now `await model.nouns()`. This touches `src/generate/openapi.ts`, `src/generate/sqlite.ts`, `src/generate/schema.ts`, `src/generate/xstate.ts`, `src/generate/mdxui.ts`, `src/generate/readings.ts`, `src/generate/readme.ts`, `src/generate/ilayer.ts`.

- [ ] **Step 5: Run all tests**

Run: `npx vitest run`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add src/model/ src/generate/
git commit -m "refactor(model): make DataLoader interface async for EntityDB fan-out"
```

---

## Phase 3: CSDP Validation Pipeline

### Task 7: Extract CSDP validation as a pure function module

**Files:**
- Create: `src/csdp/validate.ts`
- Create: `src/csdp/validate.test.ts`

- [ ] **Step 1: Write failing test for arity check (CSDP Step 4)**

```typescript
describe('CSDP Step 4: arity check', () => {
  it('rejects ternary with UC spanning < n-1 roles', () => {
    const schema = {
      factTypes: [{
        id: 'ft1',
        reading: 'A has B for C',
        roles: [{ nounName: 'A' }, { nounName: 'B' }, { nounName: 'C' }],
      }],
      constraints: [{
        kind: 'UC',
        spans: [{ factTypeId: 'ft1', roleIndex: 0 }], // single-role UC on ternary
      }],
    }
    const result = validateCsdp(schema)
    expect(result.valid).toBe(false)
    expect(result.violations[0].type).toBe('arity_violation')
    expect(result.violations[0].fix).toContain('split into binaries')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/csdp/validate.test.ts`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement CSDP validation skeleton**

```typescript
// src/csdp/validate.ts
export interface CsdpViolation {
  type: 'arity_violation' | 'missing_mandatory' | 'conflicting_constraints' |
        'undeclared_noun' | 'non_elementary_fact' | 'missing_subtype_constraint' |
        'missing_ring_constraint'
  message: string
  fix: string
  factTypeId?: string
  constraintId?: string
}

export interface CsdpResult {
  valid: boolean
  violations: CsdpViolation[]
  proposedConstraints: InducedConstraint[]
}

export function validateCsdp(schema: SchemaIR, population?: Population): CsdpResult {
  const violations: CsdpViolation[] = []
  // Step 4: arity check
  // Step 5: mandatory role check
  // Step 6: subtype constraint check
  // Step 7: ring constraint check, elementarity check, completeness check
  return { valid: violations.length === 0, violations, proposedConstraints: [] }
}
```

- [ ] **Step 4: Run tests**

Run: `npx vitest run src/csdp/validate.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/csdp/validate.ts src/csdp/validate.test.ts
git commit -m "feat(csdp): add validation skeleton with arity check"
```

---

### Task 8: Add remaining CSDP validation checks

**Files:**
- Modify: `src/csdp/validate.ts`
- Modify: `src/csdp/validate.test.ts`

- [ ] **Step 1: Write failing tests for each validation type**

Tests for:
- Missing mandatory constraint (induction found MC but not declared)
- Conflicting constraints
- Undeclared noun in constraint
- Non-elementary fact (and-test)
- Missing subtype constraint (totality/exclusion)
- Missing ring constraint (self-referential binary)

- [ ] **Step 2: Implement each check**

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/csdp/validate.test.ts`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/csdp/validate.ts src/csdp/validate.test.ts
git commit -m "feat(csdp): complete validation checks (mandatory, subtype, ring, elementarity)"
```

---

### Task 9: Wire WASM induction into claims pipeline

**Files:**
- Create: `src/csdp/induce.ts`
- Create: `src/csdp/induce.test.ts`
- Modify: `src/api/router.ts` (add `/api/induce` endpoint)

- [ ] **Step 1: Write failing test for TypeScript induction wrapper**

```typescript
describe('induce wrapper', () => {
  it('calls WASM induce_from_population and returns InducedConstraints', () => {
    const ir = { factTypes: { /* ... */ }, constraints: [] }
    const population = { facts: { /* ... */ } }
    const result = induceConstraints(ir, population)
    expect(result.constraints).toBeDefined()
    expect(result.rules).toBeDefined()
  })
})
```

- [ ] **Step 2: Implement the wrapper**

The wrapper calls `load_ir(JSON.stringify(ir))` then `induce_from_population(JSON.stringify(population))` from the WASM module, parses the JSON result, and returns typed `InductionResult`.

- [ ] **Step 3: Add `/api/induce` endpoint for apis worker to call (rounds 1-2)**

- [ ] **Step 4: Run tests**

Run: `npx vitest run src/csdp/induce.test.ts`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/csdp/induce.ts src/csdp/induce.test.ts src/api/router.ts
git commit -m "feat(csdp): wire WASM induction into TypeScript with /api/induce endpoint"
```

---

## Phase 4: RMAP Formalization

### Task 10: Extract RMAP as a pure function module

**Files:**
- Create: `src/rmap/procedure.ts`
- Create: `src/rmap/procedure.test.ts`

- [ ] **Step 1: Write failing test for RMAP Step 1 (compound UC to separate table)**

```typescript
describe('RMAP Step 1', () => {
  it('maps M:N binary to separate table with compound PK', () => {
    const schema = {
      factTypes: [{ id: 'ft1', reading: 'Person speaks Language', roles: [
        { nounName: 'Person' }, { nounName: 'Language' }
      ]}],
      constraints: [{ kind: 'UC', spans: [
        { factTypeId: 'ft1', roleIndex: 0 },
        { factTypeId: 'ft1', roleIndex: 1 },
      ]}],
    }
    const tables = rmap(schema)
    expect(tables).toContainEqual({
      name: 'person_speaks_language',
      primaryKey: ['person_id', 'language_id'],
      columns: [
        { name: 'person_id', type: 'TEXT', nullable: false },
        { name: 'language_id', type: 'TEXT', nullable: false },
      ],
    })
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/rmap/procedure.test.ts`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement RMAP core**

Implement Steps 0.1-0.7 (preprocessing) and Steps 1-6 (mapping) as pure functions that take a validated SchemaIR and return a `RelationalSchema` (array of table definitions with columns, keys, constraints, and CHECK clauses).

- [ ] **Step 4: Run tests**

Run: `npx vitest run src/rmap/procedure.test.ts`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/rmap/procedure.ts src/rmap/procedure.test.ts
git commit -m "feat(rmap): implement formal RMAP procedure from Halpin Ch. 10"
```

---

### Task 11: Add RMAP tests for all mapping rules

**Files:**
- Modify: `src/rmap/procedure.test.ts`

- [ ] **Step 1: Write tests for each RMAP step**

Tests for:
- Step 2: functional roles grouped by entity
- Step 3: 1:1 absorption (favor fewer nulls)
- Step 4: independent entity to single-column table
- Step 5: unpack composite identifiers
- Step 6: UC to keys, MC to NOT NULL, SS to FK, value to CHECK, ring to CHECK
- Step 0.1: binarize exclusive unaries
- Step 0.3: subtype absorption

- [ ] **Step 2: Implement each step**

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/rmap/procedure.test.ts`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/rmap/procedure.ts src/rmap/procedure.test.ts
git commit -m "feat(rmap): complete all mapping rules with tests"
```

---

### Task 12: Wire RMAP into generators

**Files:**
- Modify: `src/generate/sqlite.ts`
- Modify: `src/generate/openapi.ts`
- Modify: `src/generate/sqlite.test.ts`
- Modify: `src/generate/openapi.test.ts`

- [ ] **Step 1: Write failing test that generators use RMAP output**

Test that `generateSQLite` and `generateOpenAPI` accept a `RelationalSchema` from RMAP instead of building their own ad-hoc mappings.

- [ ] **Step 2: Refactor generators to consume RMAP output**

The generators currently walk the DomainModel and build their own table/schema mappings. Refactor to accept `RelationalSchema` as input and generate DDL/OpenAPI from it.

- [ ] **Step 3: Run all generator tests**

Run: `npx vitest run src/generate/`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/generate/ src/rmap/
git commit -m "refactor(generate): wire generators to consume RMAP output"
```

---

## Phase 5: Rewrite Claims Pipeline

### Task 13: Rewrite ingestClaims to build batches

**Files:**
- Modify: `src/claims/steps.ts`
- Modify: `src/claims/ingest.ts`
- Create: `src/claims/batch-builder.ts`
- Create: `src/claims/batch-builder.test.ts`

- [ ] **Step 1: Write failing test for batch builder**

```typescript
describe('BatchBuilder', () => {
  it('accumulates entities from ingest phases', () => {
    const builder = new BatchBuilder('tickets')
    builder.addEntity({ id: 'n1', type: 'Noun', data: { name: 'Customer' } })
    builder.addEntity({ id: 'r1', type: 'Reading', data: { text: 'Customer has Name' } })
    const batch = builder.toBatch()
    expect(batch.entities).toHaveLength(2)
    expect(batch.domain).toBe('tickets')
  })
})
```

- [ ] **Step 2: Implement BatchBuilder**

A mutable accumulator that replaces `db.createInCollection()` calls in the ingest steps. Each `addEntity` adds to an internal array. `toBatch()` returns the complete `BatchEntity[]` for WAL commit.

- [ ] **Step 3: Refactor `ingestNouns`, `ingestReadings`, etc. to use BatchBuilder**

Replace `await db.createInCollection('nouns', data)` with `builder.addEntity({ id, type: 'Noun', data })`. The hooks (`readingAfterCreate`, etc.) add their derived entities to the same builder.

- [ ] **Step 4: Run existing pipeline tests**

Run: `npx vitest run src/claims/ src/pipeline.test.ts src/pipeline.university.test.ts`
Expected: ALL PASS (same behavior, different write target)

- [ ] **Step 5: Commit**

```bash
git add src/claims/
git commit -m "refactor(claims): replace db.createInCollection with BatchBuilder"
```

---

### Task 14: Add CSDP validation to claims endpoint

**Files:**
- Modify: `src/api/router.ts` (the `/api/claims` handler)
- Create: `src/csdp/pipeline.ts`
- Create: `src/csdp/pipeline.test.ts`

- [ ] **Step 1: Write failing test for CSDP pipeline integration**

```typescript
describe('CSDP pipeline', () => {
  it('rejects invalid schema with proposed fixes', async () => {
    const claims = {
      nouns: [{ name: 'A' }, { name: 'B' }, { name: 'C' }],
      readings: [{ text: 'A has B for C' }],
      constraints: [{ kind: 'UC', text: 'Each A has at most one B.' }], // single-role UC on ternary
    }
    const result = await runCsdpPipeline(claims)
    expect(result.valid).toBe(false)
    expect(result.violations[0].type).toBe('arity_violation')
  })

  it('accepts valid schema and returns batch', async () => {
    const claims = {
      nouns: [{ name: 'Customer', objectType: 'entity' }, { name: 'Name', objectType: 'value' }],
      readings: [{ text: 'Customer has Name' }],
      constraints: [{ kind: 'UC', text: 'Each Customer has at most one Name.' }],
    }
    const result = await runCsdpPipeline(claims)
    expect(result.valid).toBe(true)
    expect(result.batch.entities.length).toBeGreaterThan(0)
  })
})
```

- [ ] **Step 2: Implement the CSDP pipeline orchestrator**

Wires together: parse claims -> build schema IR -> induction round 3 -> validateCsdp -> if valid, build batch via BatchBuilder -> RMAP -> return batch + relational schema.

- [ ] **Step 3: Wire into `/api/claims` endpoint**

Replace the current `ingestClaims()` call with `runCsdpPipeline()`. On validation failure, return 422 with violations and proposed fixes. On success, commit batch to WAL and materialize.

- [ ] **Step 4: Run tests**

Run: `npx vitest run src/csdp/pipeline.test.ts`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/csdp/ src/api/router.ts
git commit -m "feat(csdp): integrate CSDP validation into claims endpoint"
```

---

## Phase 6: Batch Materializer

### Task 15: Build fan-out materializer

**Files:**
- Create: `src/worker/materialize.ts`
- Create: `src/worker/materialize.test.ts`

- [ ] **Step 1: Write failing test for materialization**

```typescript
describe('materializeBatch', () => {
  it('creates EntityDB DOs and indexes in RegistryDB', async () => {
    const batch = {
      entities: [
        { id: 'n1', type: 'Noun', domain: 'tickets', data: { name: 'Customer' } },
        { id: 'r1', type: 'Reading', domain: 'tickets', data: { text: 'Customer has Name' } },
      ]
    }
    const entityStubs = new Map()
    const registryStub = { indexEntity: vi.fn(), indexNoun: vi.fn() }
    await materializeBatch(batch, (id) => entityStubs.get(id) || createMockStub(entityStubs, id), registryStub)
    expect(entityStubs.size).toBe(2)
    expect(registryStub.indexEntity).toHaveBeenCalledTimes(2)
  })
})
```

- [ ] **Step 2: Implement materializeBatch**

For each entity in the batch:
1. Get or create EntityDB DO stub via `env.ENTITY_DB.idFromName(entity.id)`
2. Call `stub.put({ id, type, data })`
3. Call `registry.indexEntity(type, id, domain)`
4. For Noun entities, also call `registry.indexNoun(name, domain)`

Fan-out in batches of 50 (same pattern as existing `fanOutCollect` in `src/worker/query.ts`).

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/worker/materialize.test.ts`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/worker/materialize.ts src/worker/materialize.test.ts
git commit -m "feat(worker): add batch materializer for EntityDB DO fan-out"
```

---

## Phase 7: Router Rewrite

### Task 16: Add entity-type endpoints alongside existing collection endpoints

**Files:**
- Modify: `src/api/router.ts`
- Create: `src/api/entity-routes.ts`
- Create: `src/api/entity-routes.test.ts`

- [ ] **Step 1: Write failing test for new entity-type list endpoint**

```typescript
describe('GET /api/entities/:type', () => {
  it('returns entities by type from Registry fan-out', async () => {
    // Mock RegistryDB.getEntityIds and EntityDB.get
    // Verify response shape matches current collection response
  })
})
```

- [ ] **Step 2: Implement entity-type routes**

New routes that coexist with existing collection routes:
- `GET /api/entities/:type?domain=X` — list by type + domain
- `GET /api/entities/:type/:id` — get by ID
- `POST /api/entities/:type` — create (goes through CSDP for metamodel types)
- `PATCH /api/entities/:type/:id` — update
- `DELETE /api/entities/:type/:id` — soft-delete + cascade

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/api/entity-routes.test.ts`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/api/entity-routes.ts src/api/entity-routes.test.ts src/api/router.ts
git commit -m "feat(api): add entity-type endpoints alongside collection endpoints"
```

---

### Task 17: Migrate router from collection endpoints to entity-type endpoints

**Files:**
- Modify: `src/api/router.ts`

- [ ] **Step 1: Redirect collection endpoints to entity-type endpoints**

The generic `GET/POST/PATCH/DELETE /api/:collection` routes now resolve the collection slug to an entity type name and delegate to the entity-type route handlers. This provides backward compatibility while the apis worker transitions.

- [ ] **Step 2: Run all existing tests**

Run: `npx vitest run`
Expected: ALL PASS

- [ ] **Step 3: Commit**

```bash
git add src/api/router.ts
git commit -m "refactor(api): redirect collection endpoints to entity-type handlers"
```

---

### Task 17b: Implement cascade deletes via batch WAL

**Files:**
- Create: `src/worker/cascade.ts`
- Create: `src/worker/cascade.test.ts`
- Modify: `src/api/entity-routes.ts`

- [ ] **Step 1: Write failing test for cascade delete**

```typescript
describe('cascade delete', () => {
  it('discovers dependents from Registry and builds delete batch', async () => {
    // Deleting a Noun should cascade to its Readings, Roles, Constraints
    const registry = {
      getEntityIds: vi.fn()
        .mockResolvedValueOnce(['r1', 'r2'])  // Readings referencing this noun
        .mockResolvedValueOnce(['role1'])       // Roles referencing this noun
    }
    const deleteBatch = await buildCascadeDeleteBatch('n1', 'Noun', registry, entityStubGetter)
    expect(deleteBatch.entities.map(e => e.id)).toContain('n1')
    expect(deleteBatch.entities.map(e => e.id)).toContain('r1')
    expect(deleteBatch.entities.map(e => e.id)).toContain('r2')
    expect(deleteBatch.entities.map(e => e.id)).toContain('role1')
  })
})
```

- [ ] **Step 2: Implement cascade delete**

Cascade logic derived from the metamodel readings: "Graph Schema has Reading" means deleting a Graph Schema cascades to its Readings. The dependency graph is built by reading the metamodel's fact types and finding which entity types reference the target type.

- [ ] **Step 3: Wire into DELETE entity-routes endpoint**

- [ ] **Step 4: Run tests**

Run: `npx vitest run src/worker/cascade.test.ts`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/worker/cascade.ts src/worker/cascade.test.ts src/api/entity-routes.ts
git commit -m "feat(worker): implement cascade deletes via batch WAL"
```

---

### Task 17c: Implement depth population for entity references

**Files:**
- Modify: `src/api/entity-routes.ts`
- Modify: `src/api/entity-routes.test.ts`

- [ ] **Step 1: Write failing test for depth population**

```typescript
describe('depth population', () => {
  it('resolves entity references in data blobs via second fan-out', async () => {
    // Entity has data: { graphSchemaId: 'gs1' }
    // With depth=1, gs1 is resolved to its full entity data
  })
})
```

- [ ] **Step 2: Implement depth resolution**

When `?depth=1` (or higher) is requested, scan entity data blobs for fields ending in `Id`. For each, do a secondary fan-out to resolve the referenced entity. Replace the ID with the full entity object.

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/api/entity-routes.test.ts`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/api/entity-routes.ts src/api/entity-routes.test.ts
git commit -m "feat(api): implement depth population via secondary entity fan-out"
```

---

### Task 17d: Migrate CDC aggregation to Worker layer

**Files:**
- Modify: `src/api/router.ts` (WebSocket handler)
- Create: `src/worker/cdc.ts`
- Create: `src/worker/cdc.test.ts`

- [ ] **Step 1: Write failing test for batch-triggered CDC**

```typescript
describe('CDC on batch commit', () => {
  it('broadcasts entity changes from committed batch', () => {
    const batch = { entities: [{ id: 'n1', type: 'Noun', domain: 'tickets', data: {} }] }
    const events = buildCdcEvents(batch, 'create')
    expect(events).toHaveLength(1)
    expect(events[0]).toEqual({ entityId: 'n1', type: 'Noun', operation: 'create', domain: 'tickets' })
  })
})
```

- [ ] **Step 2: Implement CDC event builder and WebSocket broadcast trigger**

After `materializeBatch` succeeds, build CDC events from the batch entities and broadcast via the existing WebSocket mechanism. The trigger is batch commit, not individual entity writes.

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/worker/cdc.test.ts`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/worker/cdc.ts src/worker/cdc.test.ts src/api/router.ts
git commit -m "feat(worker): migrate CDC aggregation to Worker layer, trigger on batch commit"
```

---

## Phase 8: Delete Payload Layer

### Task 18: Delete Payload legacy code

**Files:**
- Delete: `src/do-adapter.ts`
- Delete: `src/do-adapter.test.ts`
- Delete: `src/wipe-tables.ts`
- Modify: `src/collections.ts` (gut to minimal type-name mapping only)
- Modify: `src/api/collections.ts` (delete parsePayloadWhereParams)
- Modify: `src/domain-do.ts` (delete query engine, keep batch WAL + generators)
- Modify: `src/index.ts` (update exports)

- [ ] **Step 1: Delete files and remove dead imports**

- [ ] **Step 2: Run all tests to verify nothing breaks**

Run: `npx vitest run`
Expected: ALL PASS (if any fail, there are still references to deleted code that need updating)

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "refactor: delete Payload CMS abstraction layer"
```

---

### Task 19: Delete bootstrap DDL

**Files:**
- Modify: `src/schema/bootstrap.ts` (delete hardcoded DDL)
- Modify: `src/schema/index.ts`
- Modify: `src/domain-do.ts` (remove initDomainSchema that runs BOOTSTRAP_DDL)

- [ ] **Step 1: Remove bootstrap DDL — schema is now generated from readings via RMAP**

- [ ] **Step 2: Run all tests**

Run: `npx vitest run`
Expected: ALL PASS

- [ ] **Step 3: Commit**

```bash
git add src/schema/ src/domain-do.ts
git commit -m "refactor: remove hardcoded bootstrap DDL — schema generated from readings"
```

---

## Phase 9: Migration

### Task 20: Add wrangler migration v4

**Files:**
- Modify: `wrangler.jsonc`

- [ ] **Step 1: Add v4 migration tag**

```jsonc
{
  "migrations": [
    { "tag": "v1", "new_sqlite_classes": ["GraphDLDB"] },
    { "tag": "v2", "new_sqlite_classes": ["EntityDB", "DomainDB", "RegistryDB"] },
    { "tag": "v3", "deleted_classes": ["GraphDLDB"] },
    { "tag": "v4" }
  ]
}
```

v4 is a no-op migration tag — the DomainDB DO class still exists but its `ensureInit` now creates only `batches` and `generators` tables instead of 20+ metamodel tables. Existing data in the old tables is ignored (will be re-ingested from readings).

- [ ] **Step 2: Commit**

```bash
git add wrangler.jsonc
git commit -m "chore: add v4 migration tag for DomainDB reduction"
```

---

### Task 21: Seed core domain

**Files:**
- Create: `scripts/seed-core.ts`

- [ ] **Step 1: Write seed script**

Script that reads all `readings/*.md` files and POSTs them to the `/api/claims` endpoint (which now runs the full CSDP pipeline). This seeds the metamodel as EntityDB DOs.

```typescript
// scripts/seed-core.ts
import { readFileSync, readdirSync } from 'fs'

const READINGS_DIR = './readings'
const GRAPHDL_URL = process.env.GRAPHDL_URL || 'http://localhost:8787'

async function seedCore() {
  const files = readdirSync(READINGS_DIR).filter(f => f.endsWith('.md'))
  for (const file of files) {
    const text = readFileSync(`${READINGS_DIR}/${file}`, 'utf-8')
    const domain = file.replace('.md', '')
    console.log(`Seeding ${domain}...`)
    const res = await fetch(`${GRAPHDL_URL}/parse`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text, domain }),
    })
    const claims = await res.json()
    const ingestRes = await fetch(`${GRAPHDL_URL}/api/claims`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...claims, domain }),
    })
    const result = await ingestRes.json()
    if (!ingestRes.ok) {
      console.error(`  REJECTED: ${JSON.stringify(result.violations, null, 2)}`)
    } else {
      console.log(`  OK: ${result.entityCount} entities`)
    }
  }
}

seedCore()
```

- [ ] **Step 2: Test locally against dev wrangler**

Run: `npx wrangler dev` in one terminal
Run: `npx tsx scripts/seed-core.ts` in another
Expected: All 7 reading files seed successfully

- [ ] **Step 3: Commit**

```bash
git add scripts/seed-core.ts
git commit -m "feat: add core domain seed script"
```

---

## Phase 10: Progressive Induction in APIs

> **Note:** This phase targets the `apis` repository at `C:/Users/lippe/Repos/apis/`, not graphdl-orm. It depends on the `/api/induce` endpoint created in Task 9.

### Task 22: Add induction rounds 1-2 to apis worker

**Files (in apis repo):**
- Modify: `C:/Users/lippe/Repos/apis/graphdl/extract-claims.ts`

- [ ] **Step 1: After deterministic parse, call graphdl-orm `/api/induce`**

After `parseFORML2` returns, if there are instance facts, call `/api/induce` with the population. Add induced constraints to `constraints_1`.

- [ ] **Step 2: Pass `constraints_1` as LLM context**

Add discovered constraints to the system prompt for LLM extraction. The LLM sees "The system already discovered these constraints: [list]" and can validate or refine.

- [ ] **Step 3: After merge, call `/api/induce` again with merged population**

After merging deterministic + LLM claims, run induction round 2. Merge induced constraints into the claims before sending to graphdl-orm `/api/claims`.

- [ ] **Step 4: Test the full pipeline**

Test end-to-end: natural language text -> deterministic parse -> induction round 1 -> LLM -> merge -> induction round 2 -> claims endpoint -> CSDP validation (induction round 3).

- [ ] **Step 5: Commit**

```bash
git add graphdl/extract-claims.ts
git commit -m "feat(apis): add progressive induction rounds 1-2 to extraction pipeline"
```
