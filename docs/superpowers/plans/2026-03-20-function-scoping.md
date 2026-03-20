# Function Scoping Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable app-scoped multi-domain ingestion by decomposing the ingestion pipeline into composable steps with a shared resolution scope, and re-model Noun as a subtype of Function.

**Architecture:** Extract the 6 inline steps of `ingestClaims()` into standalone functions that operate on a shared `Scope` object. Compose them two ways: `ingestClaims()` (single-domain, backward-compatible wrapper) and `ingestProject()` (multi-domain, runs each step across all domains before advancing). Update the metamodel readings to make Noun a subtype of Function with a Scope value type.

**Tech Stack:** TypeScript, Vitest, FORML2 readings, SQLite bootstrap generation

**Spec:** `docs/superpowers/specs/2026-03-20-function-scoping-design.md`

---

## File Structure

| File | Responsibility |
|---|---|
| `src/claims/scope.ts` (new) | `Scope` type, `createScope()`, `resolveNoun()`, `resolveSchema()` |
| `src/claims/scope.test.ts` (new) | Scope resolution tests (local → app → org → public cascade) |
| `src/claims/steps.ts` (new) | 6 extracted step functions: `ingestNouns`, `ingestSubtypes`, `ingestReadings`, `ingestConstraints`, `ingestTransitions`, `ingestFacts` |
| `src/claims/steps.test.ts` (new) | Step function unit tests |
| `src/claims/ingest.ts` (modify) | `ingestClaims()` becomes thin wrapper over steps. New `ingestProject()`. Remove `resolveNounAcrossDomains`, `resolveReadingAcrossDomains`. |
| `src/claims/ingest.test.ts` (unchanged) | Existing tests must pass as-is (regression) |
| `src/claims/ingest-project.test.ts` (new) | `ingestProject` unit tests (cross-domain resolution, error handling) |
| `src/claims/ingest-project-integration.test.ts` (new) | Integration test with `support.auto.dev` domain files |
| `readings/core.md` (modify) | Function as supertype, Noun as subtype. Add Scope value type. |
| `readings/state.md` (modify) | Remove `Function(.id) is an entity type.` |
| `readings/organizations.md` (modify) | Add Domain visibility derivation rules |
| `src/schema/bootstrap.ts` (regenerate) | Run `npx tsx scripts/generate-bootstrap.ts` |
| `src/api/seed.ts` (modify) | Switch from per-domain `ingestClaims` loop to single `ingestProject` call |
| `src/api/claims.ts` (modify) | Switch from per-domain `ingestClaims` loop to single `ingestProject` call |

**Note on scope resolution simplification:** The `resolveNoun` function implements a 2-level lookup (local domain, then any domain in scope) rather than the full 4-level cascade (local → app → org → public). This is sufficient because `ingestProject` only pools domains within the same project/app — the scope already contains only visible domains. Full cascade filtering can be added later if needed.

---

### Task 1: Create Scope Type and Resolution Helpers

**Files:**
- Create: `src/claims/scope.ts`
- Create: `src/claims/scope.test.ts`

- [ ] **Step 1: Write failing test for createScope**

```typescript
// src/claims/scope.test.ts
import { describe, it, expect } from 'vitest'
import { createScope } from './scope'

describe('createScope', () => {
  it('creates an empty scope', () => {
    const scope = createScope()
    expect(scope.nouns.size).toBe(0)
    expect(scope.schemas.size).toBe(0)
    expect(scope.skipped).toBe(0)
    expect(scope.errors).toEqual([])
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/claims/scope.test.ts`
Expected: FAIL — `createScope` not found

- [ ] **Step 3: Implement createScope**

```typescript
// src/claims/scope.ts
export interface NounRecord {
  id: string
  name: string
  domainId: string
  [key: string]: any
}

export interface SchemaRecord {
  id: string
  [key: string]: any
}

export interface Scope {
  /** domainId:nounName -> noun record */
  nouns: Map<string, NounRecord>
  /** reading text -> graph schema record */
  schemas: Map<string, SchemaRecord>
  /** count of items skipped due to idempotency (already exists) */
  skipped: number
  /** accumulated errors */
  errors: string[]
}

export function createScope(): Scope {
  return {
    nouns: new Map(),
    schemas: new Map(),
    skipped: 0,
    errors: [],
  }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/claims/scope.test.ts`
Expected: PASS

- [ ] **Step 5: Write failing test for addNoun and resolveNoun (local resolution)**

```typescript
// append to src/claims/scope.test.ts
import { addNoun, resolveNoun } from './scope'

describe('resolveNoun', () => {
  it('resolves noun from local domain first', () => {
    const scope = createScope()
    addNoun(scope, { id: 'n1', name: 'Status', domainId: 'd1' })
    addNoun(scope, { id: 'n2', name: 'Status', domainId: 'd2' })

    const result = resolveNoun(scope, 'Status', 'd1')
    expect(result).toBeDefined()
    expect(result!.id).toBe('n1')
  })

  it('returns null for unresolvable noun', () => {
    const scope = createScope()
    const result = resolveNoun(scope, 'Missing', 'd1')
    expect(result).toBeNull()
  })
})
```

- [ ] **Step 6: Run test to verify it fails**

Run: `npx vitest run src/claims/scope.test.ts`
Expected: FAIL — `addNoun`, `resolveNoun` not found

- [ ] **Step 7: Implement addNoun and resolveNoun**

```typescript
// append to src/claims/scope.ts

/** Add a noun to the scope, keyed by domainId:name */
export function addNoun(scope: Scope, noun: NounRecord): void {
  scope.nouns.set(`${noun.domainId}:${noun.name}`, noun)
}

/**
 * Resolve a noun by name within the scope.
 * Search order: local domain first, then all other domains.
 * App/org scoping is applied by the caller (ingestProject pools
 * only domains within the same app).
 */
export function resolveNoun(
  scope: Scope,
  name: string,
  domainId: string,
): NounRecord | null {
  // 1. Local domain
  const local = scope.nouns.get(`${domainId}:${name}`)
  if (local) return local

  // 2. Any domain in scope (scope only contains visible domains)
  for (const [_key, noun] of scope.nouns) {
    if (noun.name === name) return noun
  }

  return null
}
```

- [ ] **Step 8: Run test to verify it passes**

Run: `npx vitest run src/claims/scope.test.ts`
Expected: PASS

- [ ] **Step 9: Write failing test for addSchema and resolveSchema**

```typescript
// append to src/claims/scope.test.ts
import { addSchema, resolveSchema } from './scope'

describe('resolveSchema', () => {
  it('resolves schema by reading text', () => {
    const scope = createScope()
    addSchema(scope, 'Customer has Name', { id: 'gs1' })

    const result = resolveSchema(scope, 'Customer has Name')
    expect(result).toBeDefined()
    expect(result!.id).toBe('gs1')
  })

  it('returns null for missing schema', () => {
    const scope = createScope()
    const result = resolveSchema(scope, 'Missing reading')
    expect(result).toBeNull()
  })
})
```

- [ ] **Step 10: Run test to verify it fails**

Run: `npx vitest run src/claims/scope.test.ts`
Expected: FAIL — `addSchema`, `resolveSchema` not found

- [ ] **Step 11: Implement addSchema and resolveSchema**

```typescript
// append to src/claims/scope.ts

export function addSchema(scope: Scope, readingText: string, schema: SchemaRecord): void {
  scope.schemas.set(readingText, schema)
}

export function resolveSchema(scope: Scope, readingText: string): SchemaRecord | null {
  return scope.schemas.get(readingText) || null
}
```

- [ ] **Step 12: Run test to verify it passes**

Run: `npx vitest run src/claims/scope.test.ts`
Expected: PASS

- [ ] **Step 13: Run full test suite for regression**

Run: `npm test`
Expected: All 466+ tests pass

- [ ] **Step 14: Commit**

```bash
git add src/claims/scope.ts src/claims/scope.test.ts
git commit -m "feat: add Scope type and resolution helpers for multi-domain ingestion"
```

---

### Task 2: Extract Step Functions from ingestClaims

**Files:**
- Create: `src/claims/steps.ts`
- Create: `src/claims/steps.test.ts`

- [ ] **Step 1: Write failing test for ingestNouns step**

```typescript
// src/claims/steps.test.ts
import { describe, it, expect, vi } from 'vitest'
import { ingestNouns } from './steps'
import { createScope } from './scope'

function mockDb() {
  const store: Record<string, any[]> = {}
  let idCounter = 0
  return {
    store,
    findInCollection: vi.fn(async (collection: string, where: any, opts?: any) => {
      const all = store[collection] || []
      const filtered = all.filter((doc: any) => {
        for (const [key, cond] of Object.entries(where)) {
          if (typeof cond === 'object' && cond !== null && 'equals' in (cond as any)) {
            const fieldVal = key === 'domain' ? doc.domain : doc[key]
            if (fieldVal !== (cond as any).equals) return false
          }
        }
        return true
      })
      return { docs: filtered, totalDocs: filtered.length }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      const doc = { id: `id-${++idCounter}`, ...body }
      if (!store[collection]) store[collection] = []
      store[collection].push(doc)
      return doc
    }),
    updateInCollection: vi.fn(async (collection: string, id: string, updates: any) => {
      const coll = store[collection] || []
      const doc = coll.find((d: any) => d.id === id)
      if (doc) Object.assign(doc, updates)
      return doc
    }),
    createEntity: vi.fn(async (domainId: string, nounName: string, fields: any, reference?: string) => {
      const doc = { id: `entity-${++idCounter}`, domain: domainId, noun: nounName, reference, ...fields }
      const key = `entities_${nounName}`
      if (!store[key]) store[key] = []
      store[key].push(doc)
      return doc
    }),
    applySchema: vi.fn(async () => ({ tableMap: {}, fieldMap: {} })),
  }
}

describe('ingestNouns', () => {
  it('creates nouns and adds them to scope', async () => {
    const db = mockDb()
    const scope = createScope()
    const nouns = [
      { name: 'Customer', objectType: 'entity' as const },
      { name: 'Name', objectType: 'value' as const, valueType: 'string' },
    ]

    const count = await ingestNouns(db as any, nouns, 'd1', scope)

    expect(count).toBe(2)
    expect(scope.nouns.size).toBe(2)
    expect(scope.nouns.get('d1:Customer')).toBeDefined()
    expect(scope.nouns.get('d1:Name')).toBeDefined()
    expect(db.store.nouns).toHaveLength(2)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/claims/steps.test.ts`
Expected: FAIL — `ingestNouns` not found

- [ ] **Step 3: Extract ingestNouns from ingest.ts**

Extract Step 1 (lines 195-221 of `src/claims/ingest.ts`) into `src/claims/steps.ts`. The function takes the same inputs but writes to the scope instead of a local `nounMap`.

```typescript
// src/claims/steps.ts
import type { GraphDLDB } from '../do'
import type { ExtractedClaims } from './ingest'
import type { Scope } from './scope'
import { addNoun, addSchema, resolveNoun, resolveSchema } from './scope'
import { tokenizeReading } from './tokenize'
import { parseMultiplicity, applyConstraints } from './constraints'

const OPEN_WORLD_NOUNS = ['Right', 'Freedom', 'Liberty', 'Protection', 'Privilege']

/** Ensure a noun exists for this domain; return the doc. */
async function ensureNoun(
  db: GraphDLDB,
  name: string,
  data: Record<string, any>,
  domainId: string,
): Promise<Record<string, any>> {
  const existing = await db.findInCollection('nouns', {
    name: { equals: name },
    domain: { equals: domainId },
  }, { limit: 1 })

  if (existing.docs.length) {
    const doc = existing.docs[0]
    const updates: Record<string, any> = {}
    if (data.objectType && doc.objectType !== data.objectType) updates.objectType = data.objectType
    if (data.enumValues && !doc.enumValues) updates.enumValues = data.enumValues
    if (data.valueType && !doc.valueType) updates.valueType = data.valueType
    if (Object.keys(updates).length) {
      return (await db.updateInCollection('nouns', doc.id as string, updates))!
    }
    return doc
  }

  return db.createInCollection('nouns', { name, domain: domainId, ...data })
}

export async function ingestNouns(
  db: GraphDLDB,
  nouns: ExtractedClaims['nouns'],
  domainId: string,
  scope: Scope,
): Promise<number> {
  let count = 0
  for (const noun of nouns) {
    try {
      const data: Record<string, any> = { objectType: noun.objectType }
      if (noun.plural) data.plural = noun.plural
      if (noun.valueType) data.valueType = noun.valueType
      if (noun.format) data.format = noun.format
      const enumVals = noun.enumValues || noun.enum
      if (enumVals) data.enumValues = Array.isArray(enumVals) ? enumVals.join(', ') : enumVals
      if (noun.minimum !== undefined) data.minimum = noun.minimum
      if (noun.maximum !== undefined) data.maximum = noun.maximum
      if (noun.pattern) data.pattern = noun.pattern
      if (noun.worldAssumption) {
        data.worldAssumption = noun.worldAssumption
      } else if (OPEN_WORLD_NOUNS.some(ow => noun.name === ow || noun.name.endsWith(` ${ow}`))) {
        data.worldAssumption = 'open'
      }

      const doc = await ensureNoun(db, noun.name, data, domainId)
      addNoun(scope, { id: doc.id as string, name: noun.name, domainId, ...doc })
      count++
    } catch (err: any) {
      scope.errors.push(`[${domainId}] noun "${noun.name}": ${err.message}`)
    }
  }
  return count
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/claims/steps.test.ts`
Expected: PASS

- [ ] **Step 5: Write failing test for ingestSubtypes step**

```typescript
// append to src/claims/steps.test.ts
import { ingestSubtypes } from './steps'

describe('ingestSubtypes', () => {
  it('links child noun to parent via scope', async () => {
    const db = mockDb()
    const scope = createScope()

    // Pre-populate scope with nouns
    scope.nouns.set('d1:Resource', { id: 'n1', name: 'Resource', domainId: 'd1' })
    scope.nouns.set('d1:Request', { id: 'n2', name: 'Request', domainId: 'd1' })

    await ingestSubtypes(
      db as any,
      [{ child: 'Request', parent: 'Resource' }],
      'd1',
      scope,
    )

    expect(db.updateInCollection).toHaveBeenCalledWith(
      'nouns', 'n2', expect.objectContaining({ superType: 'n1' })
    )
  })
})
```

- [ ] **Step 6: Run test to verify it fails, then implement ingestSubtypes**

Extract Step 2 (lines 223-243 of `ingest.ts`) into `steps.ts`. Uses `resolveNoun(scope, ...)` instead of `resolveNounAcrossDomains(db, ...)`.

- [ ] **Step 7: Run test to verify it passes**

Run: `npx vitest run src/claims/steps.test.ts`

- [ ] **Step 8: Write failing test for ingestReadings step**

Test that it creates graph schemas, readings, roles, adds to scope, and applies multiplicity constraints. Extract Step 3 (lines 245-345 of `ingest.ts`). **Important:** when a reading already exists (idempotency), increment `scope.skipped++` (current lines 271, 293). Test the idempotency case explicitly.

- [ ] **Step 9: Implement ingestReadings and verify test passes**

- [ ] **Step 10: Write failing test for ingestConstraints step**

Test that it finds host reading from scope, creates constraints+spans. Extract Step 4 (lines 347-431 of `ingest.ts`). **Important:** when a constraint already exists (idempotency check at line 406), increment `scope.skipped++`. Test the idempotency case explicitly.

- [ ] **Step 11: Implement ingestConstraints and verify test passes**

- [ ] **Step 12: Write failing test for ingestTransitions step**

Test that it creates state machine definitions, statuses, event types, transitions. Uses `resolveNoun(scope, ...)` instead of `resolveNounAcrossDomains`. Extract Step 5 (lines 433-530 of `ingest.ts`).

- [ ] **Step 13: Implement ingestTransitions and verify test passes**

- [ ] **Step 14: Write failing test for ingestFacts step**

Test that it normalizes fact format, calls `createEntity` with correct camelCase field names. Extract Step 6 (lines 532-600 of `ingest.ts`). Note: the `applySchema` call is NOT inside `ingestFacts` — it's handled by the caller (`ingestClaims` or `ingestProject`).

- [ ] **Step 15: Implement ingestFacts and verify test passes**

- [ ] **Step 16: Run full test suite for regression**

Run: `npm test`
Expected: All tests pass (existing tests still import from `ingest.ts` which hasn't changed yet)

- [ ] **Step 17: Commit**

```bash
git add src/claims/steps.ts src/claims/steps.test.ts
git commit -m "feat: extract 6 ingestion steps into standalone composable functions"
```

---

### Task 3: Rewrite ingestClaims as Thin Wrapper

**Files:**
- Modify: `src/claims/ingest.ts`
- Unchanged: `src/claims/ingest.test.ts` (must still pass)

- [ ] **Step 1: Run existing ingest tests to establish baseline**

Run: `npx vitest run src/claims/ingest.test.ts`
Expected: All 11 tests pass

- [ ] **Step 2: Rewrite ingestClaims to use step functions**

Replace the inline steps in `ingestClaims()` with calls to the extracted step functions from `steps.ts`. Remove `resolveNounAcrossDomains` and `resolveReadingAcrossDomains` (their behavior is now in scope resolution). Keep `ensureNoun` as an internal helper if `steps.ts` needs it, or remove if fully migrated.

```typescript
// src/claims/ingest.ts — new ingestClaims
import { createScope } from './scope'
import {
  ingestNouns, ingestSubtypes, ingestReadings,
  ingestConstraints, ingestTransitions, ingestFacts,
} from './steps'

export async function ingestClaims(
  db: GraphDLDB,
  opts: { claims: ExtractedClaims; domainId: string },
): Promise<IngestClaimsResult> {
  const { claims, domainId } = opts
  const scope = createScope()

  const nouns = await ingestNouns(db, claims.nouns, domainId, scope)
  await ingestSubtypes(db, claims.subtypes || [], domainId, scope)
  const readings = await ingestReadings(db, claims.readings, domainId, scope)
  await ingestConstraints(db, claims.constraints || [], domainId, scope)
  const stateMachines = await ingestTransitions(db, claims.transitions || [], domainId, scope)

  // Apply schema before facts (same as current lazy behavior)
  if (claims.facts?.length) {
    try { await (db as any).applySchema(domainId) } catch {}
  }
  await ingestFacts(db, claims.facts || [], domainId, scope)

  // Build result from scope (backward-compatible return type)
  return {
    nouns,
    readings,
    stateMachines,
    skipped: scope.skipped,
    errors: [...scope.errors],
  }
}
```

- [ ] **Step 3: Run existing ingest tests to verify backward compatibility**

Run: `npx vitest run src/claims/ingest.test.ts`
Expected: All 11 tests pass unchanged

- [ ] **Step 4: Run full test suite**

Run: `npm test`
Expected: All 466+ tests pass

- [ ] **Step 5: Commit**

```bash
git add src/claims/ingest.ts
git commit -m "refactor: rewrite ingestClaims as thin wrapper over extracted steps"
```

---

### Task 4: Add ingestProject for Multi-Domain Ingestion

**Files:**
- Modify: `src/claims/ingest.ts`
- Create: `src/claims/ingest-project.test.ts`

- [ ] **Step 1: Write failing test for ingestProject**

```typescript
// src/claims/ingest-project.test.ts
import { describe, it, expect, vi } from 'vitest'
import { ingestProject } from './ingest'
import type { ExtractedClaims } from './ingest'

// Same mockDb as Task 2 — copy the full mockDb() function from steps.test.ts

describe('ingestProject', () => {
  it('resolves cross-domain noun references via shared scope', async () => {
    const db = mockDb()

    // Domain A defines Status noun and state machine
    const domainA: ExtractedClaims = {
      nouns: [{ name: 'Status', objectType: 'entity' }],
      readings: [],
      constraints: [],
      transitions: [
        { entity: 'Status', from: 'Received', to: 'Triaging', event: 'triage' },
      ],
    }

    // Domain B references Status in instance facts
    const domainB: ExtractedClaims = {
      nouns: [
        { name: 'Display Color', objectType: 'value', valueType: 'string' },
      ],
      readings: [],
      constraints: [],
      facts: [
        { entity: 'Status', entityValue: 'Received', predicate: 'has',
          valueType: 'Display Color', value: 'blue' },
      ],
    }

    const result = await ingestProject(db as any, [
      { domainId: 'd-state', claims: domainA },
      { domainId: 'd-ui', claims: domainB },
    ])

    // applySchema should have been called for both domains
    expect(db.applySchema).toHaveBeenCalledTimes(2)

    // createEntity should have been called for the instance fact
    expect(db.createEntity).toHaveBeenCalledWith(
      'd-ui', 'Status', { displayColor: 'blue' }, 'Received'
    )

    expect(result.errors).toHaveLength(0)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/claims/ingest-project.test.ts`
Expected: FAIL — `ingestProject` not exported

- [ ] **Step 3: Implement ingestProject**

```typescript
// append to src/claims/ingest.ts

export interface ProjectResult {
  domains: Map<string, IngestClaimsResult>
  totals: { nouns: number; readings: number; stateMachines: number; errors: string[] }
}

export async function ingestProject(
  db: GraphDLDB,
  domains: Array<{ domainId: string; claims: ExtractedClaims }>,
): Promise<ProjectResult> {
  const scope = createScope()
  const perDomain = new Map<string, IngestClaimsResult>()
  const counters = new Map<string, { nouns: number; readings: number; stateMachines: number }>()

  for (const d of domains) counters.set(d.domainId, { nouns: 0, readings: 0, stateMachines: 0 })

  // Step 1: All nouns across all domains
  for (const { domainId, claims } of domains) {
    counters.get(domainId)!.nouns = await ingestNouns(db, claims.nouns, domainId, scope)
  }

  // Step 2: All subtypes
  for (const { domainId, claims } of domains) {
    await ingestSubtypes(db, claims.subtypes || [], domainId, scope)
  }

  // Step 3: All readings
  for (const { domainId, claims } of domains) {
    counters.get(domainId)!.readings = await ingestReadings(db, claims.readings, domainId, scope)
  }

  // Step 4: All constraints
  for (const { domainId, claims } of domains) {
    await ingestConstraints(db, claims.constraints || [], domainId, scope)
  }

  // Step 5: All transitions
  for (const { domainId, claims } of domains) {
    counters.get(domainId)!.stateMachines = await ingestTransitions(db, claims.transitions || [], domainId, scope)
  }

  // Step 5.5: Apply schema for ALL domains before facts
  for (const { domainId } of domains) {
    try { await (db as any).applySchema(domainId) } catch {}
  }

  // Step 6: All facts
  for (const { domainId, claims } of domains) {
    await ingestFacts(db, claims.facts || [], domainId, scope)
  }

  // Build per-domain results
  // Errors are prefixed with [domainId] by the step functions for attribution
  for (const { domainId } of domains) {
    const c = counters.get(domainId)!
    const prefix = `[${domainId}] `
    perDomain.set(domainId, {
      nouns: c.nouns,
      readings: c.readings,
      stateMachines: c.stateMachines,
      skipped: scope.skipped,
      errors: scope.errors.filter(e => e.startsWith(prefix)).map(e => e.slice(prefix.length)),
    })
  }

  return {
    domains: perDomain,
    totals: {
      nouns: [...counters.values()].reduce((s, c) => s + c.nouns, 0),
      readings: [...counters.values()].reduce((s, c) => s + c.readings, 0),
      stateMachines: [...counters.values()].reduce((s, c) => s + c.stateMachines, 0),
      errors: [...scope.errors],
    },
  }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/claims/ingest-project.test.ts`
Expected: PASS

- [ ] **Step 5: Write failing test for error on unresolvable reference**

```typescript
// append to ingest-project.test.ts
it('reports error for unresolvable noun in facts', async () => {
  const db = mockDb()
  const claims: ExtractedClaims = {
    nouns: [],
    readings: [],
    constraints: [],
    facts: [
      { entity: 'Nonexistent', entityValue: 'foo', predicate: 'has',
        valueType: 'Bar', value: 'baz' },
    ],
  }

  const result = await ingestProject(db as any, [
    { domainId: 'd1', claims },
  ])

  // Should have an error, not a silent skip
  expect(result.totals.errors.length).toBeGreaterThan(0)
})
```

- [ ] **Step 6: Run test, implement if needed, verify passes**

- [ ] **Step 7: Run full test suite**

Run: `npm test`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add src/claims/ingest.ts src/claims/ingest-project.test.ts
git commit -m "feat: add ingestProject for multi-domain app-scoped ingestion"
```

---

### Task 5: Update Metamodel Readings

**Files:**
- Modify: `readings/core.md`
- Modify: `readings/state.md`
- Modify: `readings/organizations.md`

- [ ] **Step 1: Update core.md — Function as supertype, Noun as subtype**

In `readings/core.md`, replace lines 4-9:

```
## Entity Types

Noun(.id) is an entity type.
  Graph Schema is a subtype of Noun.
  Status is a subtype of Noun.
  {Graph Schema, Status} are mutually exclusive subtypes of Noun.
```

With:

```
## Entity Types

Function(.id) is an entity type.
Noun is a subtype of Function.
  Graph Schema is a subtype of Noun.
  Status is a subtype of Noun.
  {Graph Schema, Status} are mutually exclusive subtypes of Noun.
```

- [ ] **Step 2: Add Scope value type to core.md**

After existing value types (after line 104), add:

```
Scope is a value type.
  The possible values of Scope are 'local', 'app', 'organization', 'public'.
```

- [ ] **Step 3: Add Function has Scope reading to core.md**

In the `### Function` section (after line 215), add:

```
Function has Scope.
  Each Function has at most one Scope.
```

- [ ] **Step 4: Update state.md — remove Function declaration**

Remove line 12 from `readings/state.md`:
```
Function(.id) is an entity type.
```

- [ ] **Step 5: Update organizations.md — add visibility derivation rules**

Append to the `## Derivation Rules` section:

```
Domain is visible to Domain := that Domain is the same Domain.
Domain is visible to Domain := Domain has Visibility 'public'.
Domain is visible to Domain := Domain belongs to App and that Domain belongs to the same App.
Domain is visible to Domain := Domain belongs to Organization and that Domain belongs to the same Organization.
```

- [ ] **Step 6: Regenerate bootstrap**

Run: `npx tsx scripts/generate-bootstrap.ts`
Expected: `src/schema/bootstrap.ts` updated without errors

- [ ] **Step 7: Run full test suite**

Run: `npm test`
Expected: All tests pass. Parse tests in `src/api/parse.test.ts` and `src/api/parse-orm.test.ts` may need verification since they parse the readings files.

- [ ] **Step 8: Commit**

```bash
git add readings/core.md readings/state.md readings/organizations.md src/schema/bootstrap.ts
git commit -m "model: Noun is a subtype of Function, add Scope value type and domain visibility rules"
```

---

### Task 6: Integration Test with support.auto.dev

**Files:**
- Create: `src/claims/ingest-project-integration.test.ts`

- [ ] **Step 1: Write integration test that parses all support.auto.dev domains**

```typescript
// src/claims/ingest-project-integration.test.ts
import { describe, it, expect, vi } from 'vitest'
import { ingestProject } from './ingest'
import { parseFORML2 } from '../api/parse'
import * as fs from 'fs'
import * as path from 'path'

describe('ingestProject integration: support.auto.dev', () => {
  it('ingests all domains and resolves cross-domain status display colors', async () => {
    const domainsDir = path.resolve(__dirname, '../../../support.auto.dev/domains')

    // Skip if support.auto.dev not available
    if (!fs.existsSync(domainsDir)) return

    const domainFiles = fs.readdirSync(domainsDir).filter(f => f.endsWith('.md'))
    const domains = domainFiles.map(file => {
      const text = fs.readFileSync(path.join(domainsDir, file), 'utf-8')
      const slug = `support-auto-dev-${file.replace('.md', '')}`
      const claims = parseFORML2(text, [])
      return { domainId: slug, claims }
    })

    // Use mock DB for integration test
    const db = mockDb()
    const result = await ingestProject(db as any, domains)

    // Should have no errors
    expect(result.totals.errors).toHaveLength(0)

    // Should have created nouns across multiple domains
    expect(result.totals.nouns).toBeGreaterThan(10)

    // Status display color facts should have been created
    expect(db.createEntity).toHaveBeenCalled()
    const statusCalls = db.createEntity.mock.calls.filter(
      ([_d, noun]: [string, string]) => noun === 'Status'
    )
    expect(statusCalls.length).toBeGreaterThanOrEqual(4)
  })
})
```

- [ ] **Step 2: Run test**

Run: `npx vitest run src/claims/ingest-project-integration.test.ts`
Expected: PASS (or skip if support.auto.dev not at expected path)

- [ ] **Step 3: Run full test suite one final time**

Run: `npm test`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/claims/ingest-project-integration.test.ts
git commit -m "test: add integration test for multi-domain ingestion with support.auto.dev"
```

---

### Task 7: Update Callers to Use ingestProject

**Files:**
- Modify: `src/api/seed.ts` (lines 56-63)
- Modify: `src/api/claims.ts` (lines 54-61)

- [ ] **Step 1: Read seed.ts and claims.ts to understand current batch loops**

Read `src/api/seed.ts` and `src/api/claims.ts` to find the per-domain `ingestClaims` loops.

- [ ] **Step 2: Update seed.ts — replace per-domain loop with ingestProject**

The current loop iterates over `body.domains` calling `ingestClaims` per domain. Replace with a single `ingestProject` call that passes all domains at once.

```typescript
// Before (seed.ts lines 56-63):
// for (const domain of body.domains) {
//   const result = await ingestClaims(db, { claims: domain.claims, domainId: domain.domainId })
//   results.push(result)
// }

// After:
import { ingestProject } from '../claims/ingest'
const result = await ingestProject(db, body.domains)
```

Keep the single-domain path (when `body.domains` has one entry or when called with `body.claims` directly) using `ingestClaims` for backward compatibility.

- [ ] **Step 3: Update claims.ts — same pattern**

Apply the same change to the batch loop in `src/api/claims.ts`.

- [ ] **Step 4: Run full test suite**

Run: `npm test`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/api/seed.ts src/api/claims.ts
git commit -m "feat: switch seed and claims endpoints to use ingestProject for multi-domain batches"
```
