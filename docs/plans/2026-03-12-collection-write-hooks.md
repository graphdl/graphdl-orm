# Collection Write Hooks Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore deterministic parse-on-write behavior so creating a Reading, Constraint, Noun, or StateMachineDefinition automatically produces all associated child objects.

**Architecture:** A hook registry (`COLLECTION_HOOKS`) maps collection slugs to afterCreate functions. A shared `createWithHook()` function enables recursive hook composition. The generic POST handler in `router.ts` calls `createWithHook()` instead of `db.createInCollection()` directly.

**Tech Stack:** TypeScript, Cloudflare Workers, Durable Objects (SQLite), itty-router, vitest

**Spec:** `docs/specs/2026-03-12-collection-write-hooks-design.md`

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `src/hooks/index.ts` | Create | Hook types, `COLLECTION_HOOKS` map, `createWithHook()` |
| `src/hooks/parse-constraint.ts` | Create | Deterministic natural language constraint parser |
| `src/hooks/parse-constraint.test.ts` | Create | Unit tests for every constraint pattern |
| `src/hooks/nouns.ts` | Create | Noun afterCreate hook (subtype parsing) |
| `src/hooks/nouns.test.ts` | Create | Noun hook tests |
| `src/hooks/readings.ts` | Create | Reading afterCreate hook (tokenize, create nouns/roles/schema, delegate constraints) |
| `src/hooks/readings.test.ts` | Create | Reading hook tests |
| `src/hooks/constraints.ts` | Create | Constraint afterCreate hook (parse text, find host reading, create spans) |
| `src/hooks/constraints.test.ts` | Create | Constraint hook tests |
| `src/hooks/state-machines.ts` | Create | StateMachineDefinition afterCreate hook |
| `src/hooks/state-machines.test.ts` | Create | SM hook tests |
| `src/do.ts` | Modify | Add migrations: `text` column on constraints, `RC` kind CHECK |
| `src/api/router.ts` | Modify | POST handler calls `createWithHook()` instead of `db.createInCollection()` |
| `src/claims/constraints.ts` | Modify | Export `ConstraintDef` type, keep existing functions unchanged |

---

## Chunk 1: Schema Migrations + Hook Infrastructure

### Task 1: Schema migrations

**Files:**
- Modify: `src/do.ts:179-195` (migrations array)

- [ ] **Step 1: Add migrations to `initTables()`**

In `src/do.ts`, add to the `migrations` array:

```typescript
// constraints: add text column for source text round-tripping
'ALTER TABLE constraints ADD COLUMN text TEXT',
```

And add a table-recreate migration for the `RC` kind (same pattern as the `org_memberships` migration at line 200-224):

```typescript
// constraints: widen kind CHECK to include 'RC' for ring constraints
// SQLite can't ALTER CHECK, so recreate if needed
```

After the existing migrations array, add:

```typescript
try {
  // Test if RC is already valid
  this.sql.exec(
    `INSERT INTO constraints (id, kind, modality, domain_id) VALUES ('__test_rc', 'RC', 'Alethic', NULL)`
  )
  this.sql.exec(`DELETE FROM constraints WHERE id = '__test_rc'`)
} catch {
  // RC not valid — recreate table with wider CHECK
  const rows = this.sql.exec(`SELECT * FROM constraints`).toArray()
  this.sql.exec(`DROP TABLE constraints`)
  this.sql.exec(`CREATE TABLE IF NOT EXISTS constraints (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('UC', 'MC', 'SS', 'XC', 'EQ', 'OR', 'XO', 'RC')),
    modality TEXT NOT NULL DEFAULT 'Alethic' CHECK (modality IN ('Alethic', 'Deontic')),
    text TEXT,
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`)
  this.sql.exec(`CREATE INDEX IF NOT EXISTS idx_constraints_domain ON constraints(domain_id)`)
  // Re-insert existing rows
  for (const row of rows) {
    const cols = Object.keys(row).filter(k => k !== 'text')
    const placeholders = cols.map(() => '?').join(', ')
    this.sql.exec(
      `INSERT INTO constraints (${cols.join(', ')}) VALUES (${placeholders})`,
      ...cols.map(c => row[c])
    )
  }
}
```

- [ ] **Step 2: Add `text` to the FIELD_MAP for constraints**

In `src/collections.ts`, verify the constraints collection field map. The `text` column name matches the Payload field name, so no mapping is needed (it passes through as-is). Verify `createInCollection` doesn't filter it out by checking `getTableColumns()` picks up the new column after migration.

- [ ] **Step 3: Run existing tests to verify no regression**

Run: `npx vitest run`
Expected: All existing tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/do.ts
git commit -m "feat: add text column and RC kind to constraints table"
```

---

### Task 2: Hook infrastructure (`src/hooks/index.ts`)

**Files:**
- Create: `src/hooks/index.ts`

- [ ] **Step 1: Create the hook registry and `createWithHook()`**

```typescript
/**
 * Collection write hooks — deterministic parse-on-write.
 *
 * Hooks run in the Worker context (not inside the DO), receiving the
 * DurableObjectStub. They call db.createInCollection(), findInCollection(),
 * etc. via RPC — same execution context as the POST handler.
 */

export interface HookResult {
  created: Record<string, any[]>
  warnings: string[]
}

export type AfterCreateHook = (
  db: any, // DurableObjectStub at runtime, typed as any for compatibility with GraphDLDB
  doc: Record<string, any>,
  context: HookContext,
) => Promise<HookResult>

export interface HookContext {
  domainId: string
  allNouns: Array<{ name: string; id: string }>
  /** When true, constraint rejection is deferred to end of batch */
  batch?: boolean
  /** Accumulator for deferred constraints in batch mode */
  deferred?: Array<{ data: Record<string, any>; error: string }>
}

export const COLLECTION_HOOKS: Record<string, AfterCreateHook> = {}

/** Merge two HookResults, combining created arrays and warnings. */
export function mergeResults(a: HookResult, b: HookResult): HookResult {
  const created = { ...a.created }
  for (const [key, docs] of Object.entries(b.created)) {
    created[key] = [...(created[key] || []), ...docs]
  }
  return { created, warnings: [...a.warnings, ...b.warnings] }
}

/** Empty result constant. */
export const EMPTY_RESULT: HookResult = { created: {}, warnings: [] }

/**
 * Create a record and run its afterCreate hook if one exists.
 * Called by the POST handler and by other hooks for recursive composition.
 */
export async function createWithHook(
  db: any,
  collection: string,
  data: Record<string, any>,
  context: HookContext,
): Promise<{ doc: Record<string, any>; hookResult: HookResult }> {
  const doc = await db.createInCollection(collection, data)
  const hook = COLLECTION_HOOKS[collection]
  if (hook) {
    const hookResult = await hook(db, doc, context)
    return { doc, hookResult }
  }
  return { doc, hookResult: EMPTY_RESULT }
}

/**
 * Refresh the allNouns list from the database.
 * Called before hook execution to ensure nouns created by prior hooks are visible.
 */
export async function refreshNouns(db: any, domainId: string): Promise<Array<{ name: string; id: string }>> {
  const result = await db.findInCollection('nouns', { domain_id: { equals: domainId } }, { limit: 0 })
  return result.docs.map((n: any) => ({ name: n.name, id: n.id }))
}

/**
 * Find-or-create pattern. Returns existing doc if found, creates if not.
 */
export async function ensure(
  db: any,
  collection: string,
  where: Record<string, any>,
  data: Record<string, any>,
): Promise<{ doc: Record<string, any>; created: boolean }> {
  const result = await db.findInCollection(collection, where, { limit: 1 })
  if (result.docs.length > 0) {
    return { doc: result.docs[0], created: false }
  }
  const doc = await db.createInCollection(collection, data)
  return { doc, created: true }
}
```

- [ ] **Step 2: Run tests**

Run: `npx vitest run`
Expected: All existing tests pass (new file has no tests yet, but shouldn't break anything).

- [ ] **Step 3: Commit**

```bash
git add src/hooks/index.ts
git commit -m "feat: add hook registry infrastructure with createWithHook"
```

---

### Task 3: Wire POST handler to use `createWithHook()`

**Files:**
- Modify: `src/api/router.ts:159-171`

- [ ] **Step 1: Import hooks and update POST handler**

Add import at top of `src/api/router.ts`:

```typescript
import { createWithHook, refreshNouns, type HookContext, COLLECTION_HOOKS } from '../hooks'
```

Replace the POST handler (lines 159-171):

```typescript
/** POST /api/:collection — create */
router.post('/api/:collection', async (request, env: Env) => {
  const { collection } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const body = await request.json() as Record<string, any>
  const db = getDB(env) as any

  // If a hook exists for this collection, use createWithHook
  if (COLLECTION_HOOKS[collection]) {
    const domainId = body.domain || ''
    const allNouns = domainId ? await refreshNouns(db, domainId) : []
    const context: HookContext = { domainId, allNouns }
    const { doc, hookResult } = await createWithHook(db, collection, body, context)
    return json({
      doc,
      message: 'Created successfully',
      ...(Object.keys(hookResult.created).length > 0 && { created: hookResult.created }),
      ...(hookResult.warnings.length > 0 && { warnings: hookResult.warnings }),
    }, { status: 201 })
  }

  // No hook — standard create
  const doc = await db.createInCollection(collection, body)
  return json({ doc, message: 'Created successfully' }, { status: 201 })
})
```

- [ ] **Step 2: Run existing tests**

Run: `npx vitest run`
Expected: All existing tests pass. The COLLECTION_HOOKS map is empty so behavior is unchanged.

- [ ] **Step 3: Commit**

```bash
git add src/api/router.ts src/hooks/index.ts
git commit -m "feat: wire POST handler to createWithHook for hook-enabled collections"
```

---

## Chunk 2: Deterministic Constraint Parser

### Task 4: `parseConstraintText()` — tests first

**Files:**
- Create: `src/hooks/parse-constraint.test.ts`

- [ ] **Step 1: Write failing tests for all constraint patterns**

```typescript
import { describe, it, expect } from 'vitest'
import { parseConstraintText } from './parse-constraint'

describe('parseConstraintText', () => {
  describe('uniqueness constraints (UC)', () => {
    it('parses "Each X has at most one Y"', () => {
      const result = parseConstraintText('Each Customer has at most one Name.')
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Customer', 'Name'] },
      ])
    })

    it('parses "Each X belongs to at most one Y"', () => {
      const result = parseConstraintText('Each Domain belongs to at most one Organization.')
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Domain', 'Organization'] },
      ])
    })

    it('parses spanning UC "For each pair of X and Y"', () => {
      const result = parseConstraintText(
        'For each pair of Widget and Widget, that Widget targets that Widget at most once.'
      )
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Widget', 'Widget'] },
      ])
    })

    it('parses ternary UC "For each combination of X and Y"', () => {
      const result = parseConstraintText(
        'For each combination of Plan and Interval, that Plan has at most one Price per that Interval.'
      )
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Plan', 'Interval', 'Price'] },
      ])
    })
  })

  describe('mandatory constraints (MC)', () => {
    it('parses "Each X has at least one Y"', () => {
      const result = parseConstraintText('Each Organization has at least one Name.')
      expect(result).toEqual([
        { kind: 'MC', modality: 'Alethic', nouns: ['Organization', 'Name'] },
      ])
    })
  })

  describe('exactly one (UC + MC)', () => {
    it('parses "Each X has exactly one Y" into two constraints', () => {
      const result = parseConstraintText('Each Section has exactly one Position.')
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Section', 'Position'] },
        { kind: 'MC', modality: 'Alethic', nouns: ['Section', 'Position'] },
      ])
    })
  })

  describe('ring constraints (RC)', () => {
    it('parses "No X [verb] itself"', () => {
      const result = parseConstraintText('No Widget targets itself.')
      expect(result).toEqual([
        { kind: 'RC', modality: 'Alethic', nouns: ['Widget'] },
      ])
    })
  })

  describe('deontic wrappers', () => {
    it('parses "It is obligatory that ..."', () => {
      const result = parseConstraintText(
        'It is obligatory that each Customer has at least one Name.'
      )
      expect(result).toEqual([
        { kind: 'MC', modality: 'Deontic', deonticOperator: 'obligatory', nouns: ['Customer', 'Name'] },
      ])
    })

    it('parses "It is forbidden that ..."', () => {
      const result = parseConstraintText(
        'It is forbidden that SupportResponse contains ProhibitedPunctuation.'
      )
      // Unrecognized inner pattern — returns null
      expect(result).toBeNull()
    })

    it('parses "It is permitted that ..."', () => {
      const result = parseConstraintText(
        'It is permitted that each SupportResponse offers Assistance.'
      )
      // "each X offers Y" doesn't match known patterns — returns null
      expect(result).toBeNull()
    })
  })

  describe('unrecognized patterns', () => {
    it('returns null for arbitrary text', () => {
      expect(parseConstraintText('This is not a constraint.')).toBeNull()
    })

    it('returns null for empty string', () => {
      expect(parseConstraintText('')).toBeNull()
    })
  })
})
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/hooks/parse-constraint.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Commit failing tests**

```bash
git add src/hooks/parse-constraint.test.ts
git commit -m "test: add failing tests for parseConstraintText"
```

---

### Task 5: `parseConstraintText()` — implementation

**Files:**
- Create: `src/hooks/parse-constraint.ts`

- [ ] **Step 1: Implement the parser**

```typescript
/**
 * Deterministic natural language constraint parser.
 *
 * Recognizes canonical FORML2 constraint patterns and returns structured
 * ParsedConstraint objects. Returns null for unrecognized text.
 *
 * Pure function — no DB or LLM dependency.
 */

export interface ParsedConstraint {
  kind: 'UC' | 'MC' | 'RC'
  modality: 'Alethic' | 'Deontic'
  deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
  nouns: string[]
}

// Match PascalCase noun names (e.g., "Customer", "SupportRequest", "APIKey")
const NOUN = '([A-Z][a-zA-Z0-9]*)'

// "Each X has/belongs to at most one Y."
const AT_MOST_ONE = new RegExp(
  `^Each ${NOUN} (?:has|belongs to|is [a-z]+ (?:by|to|in|for|of)) at most one ${NOUN}`,
  'i'
)

// "Each X has exactly one Y."
const EXACTLY_ONE = new RegExp(
  `^Each ${NOUN} (?:has|belongs to|is [a-z]+ (?:by|to|in|for|of)) exactly one ${NOUN}`,
  'i'
)

// "Each X has at least one Y."
const AT_LEAST_ONE = new RegExp(
  `^Each ${NOUN} (?:has|belongs to|is [a-z]+ (?:by|to|in|for|of)) at least one ${NOUN}`,
  'i'
)

// "For each pair of X and Y, that X ... that Y at most once."
const SPANNING_UC = new RegExp(
  `^For each pair of ${NOUN} and ${NOUN},.*at most once`,
  'i'
)

// "For each combination of X and Y, that X has at most one Z per that Y."
const TERNARY_UC = new RegExp(
  `^For each combination of ${NOUN} and ${NOUN},.*at most one ${NOUN}`,
  'i'
)

// "No X [verb] itself."
const RING_IRREFLEXIVE = new RegExp(
  `^No ${NOUN} [a-z]+ itself`,
  'i'
)

// Deontic wrappers
const DEONTIC = /^It is (obligatory|forbidden|permitted) that (.+)$/i

export function parseConstraintText(text: string): ParsedConstraint[] | null {
  if (!text || !text.trim()) return null

  const clean = text.trim().replace(/\.$/, '')

  // Check for deontic wrapper first
  const deonticMatch = clean.match(DEONTIC)
  if (deonticMatch) {
    const operator = deonticMatch[1].toLowerCase() as 'obligatory' | 'forbidden' | 'permitted'
    const inner = parseConstraintText(deonticMatch[2])
    if (!inner) return null
    return inner.map(c => ({ ...c, modality: 'Deontic' as const, deonticOperator: operator }))
  }

  // "Each X has exactly one Y" → UC + MC
  let m = clean.match(EXACTLY_ONE)
  if (m) {
    const nouns = [m[1], m[2]]
    return [
      { kind: 'UC', modality: 'Alethic', nouns },
      { kind: 'MC', modality: 'Alethic', nouns },
    ]
  }

  // "Each X has at most one Y" → UC
  m = clean.match(AT_MOST_ONE)
  if (m) {
    return [{ kind: 'UC', modality: 'Alethic', nouns: [m[1], m[2]] }]
  }

  // "Each X has at least one Y" → MC
  m = clean.match(AT_LEAST_ONE)
  if (m) {
    return [{ kind: 'MC', modality: 'Alethic', nouns: [m[1], m[2]] }]
  }

  // "For each combination of X and Y, ... at most one Z ..."
  m = clean.match(TERNARY_UC)
  if (m) {
    return [{ kind: 'UC', modality: 'Alethic', nouns: [m[1], m[2], m[3]] }]
  }

  // "For each pair of X and Y, ... at most once"
  m = clean.match(SPANNING_UC)
  if (m) {
    return [{ kind: 'UC', modality: 'Alethic', nouns: [m[1], m[2]] }]
  }

  // "No X [verb] itself"
  m = clean.match(RING_IRREFLEXIVE)
  if (m) {
    return [{ kind: 'RC', modality: 'Alethic', nouns: [m[1]] }]
  }

  return null
}
```

- [ ] **Step 2: Run tests**

Run: `npx vitest run src/hooks/parse-constraint.test.ts`
Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/hooks/parse-constraint.ts src/hooks/parse-constraint.test.ts
git commit -m "feat: add deterministic natural language constraint parser"
```

---

## Chunk 3: Noun + Reading Hooks

### Task 6: Noun hook

**Files:**
- Create: `src/hooks/nouns.ts`
- Create: `src/hooks/nouns.test.ts`
- Modify: `src/hooks/index.ts` (register hook)

- [ ] **Step 1: Write failing test**

```typescript
import { describe, it, expect, vi } from 'vitest'
import { nounAfterCreate } from './nouns'
import type { HookContext } from './index'

function mockDb(data: Record<string, any[]> = {}) {
  return {
    findInCollection: vi.fn(async (collection: string, where: any) => {
      return { docs: data[collection] || [], totalDocs: 0 }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      return { id: `new-${collection}-id`, ...body }
    }),
    updateInCollection: vi.fn(async (collection: string, id: string, body: any) => {
      return { id, ...body }
    }),
  }
}

describe('nounAfterCreate', () => {
  it('does nothing for a noun without subtype text', async () => {
    const db = mockDb()
    const doc = { id: 'n1', name: 'Customer', objectType: 'entity', domain: 'd1' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }
    const result = await nounAfterCreate(db, doc, ctx)
    expect(result.warnings).toHaveLength(0)
    expect(db.createInCollection).not.toHaveBeenCalled()
  })

  it('parses subtype text and sets superType', async () => {
    const db = mockDb({
      nouns: [{ id: 'parent-id', name: 'Request', objectType: 'entity' }],
    })
    const doc = { id: 'n1', name: 'SupportRequest', objectType: 'entity', domain: 'd1',
      promptText: 'SupportRequest is a subtype of Request' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [{ name: 'Request', id: 'parent-id' }] }
    const result = await nounAfterCreate(db, doc, ctx)
    expect(db.updateInCollection).toHaveBeenCalledWith('nouns', 'n1', { superType: 'parent-id' })
    expect(result.warnings).toHaveLength(0)
  })

  it('creates parent noun if not found', async () => {
    const db = mockDb({ nouns: [] })
    const doc = { id: 'n1', name: 'SupportRequest', objectType: 'entity', domain: 'd1',
      promptText: 'SupportRequest is a subtype of Request' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }
    const result = await nounAfterCreate(db, doc, ctx)
    expect(db.createInCollection).toHaveBeenCalledWith('nouns', expect.objectContaining({
      name: 'Request', objectType: 'entity', domain: 'd1',
    }))
    expect(result.created['nouns']).toHaveLength(1)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/hooks/nouns.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the noun hook**

```typescript
import { ensure, type HookResult, EMPTY_RESULT } from './index'

const SUBTYPE_PATTERN = /^(\S+)\s+is a subtype of\s+(\S+)/i

export async function nounAfterCreate(
  db: any,
  doc: Record<string, any>,
  context: { domainId: string; allNouns: Array<{ name: string; id: string }> },
): Promise<HookResult> {
  const text = doc.promptText || ''
  const match = text.match(SUBTYPE_PATTERN)
  if (!match) return EMPTY_RESULT

  const parentName = match[2].replace(/\.$/, '')
  const result: HookResult = { created: {}, warnings: [] }

  // Find or create the parent noun
  let parentId: string | undefined
  const existing = context.allNouns.find(n => n.name === parentName)
  if (existing) {
    parentId = existing.id
  } else {
    const { doc: parentDoc, created } = await ensure(
      db, 'nouns',
      { name: { equals: parentName }, domain_id: { equals: context.domainId } },
      { name: parentName, objectType: 'entity', domain: context.domainId },
    )
    parentId = parentDoc.id
    if (created) {
      result.created['nouns'] = [parentDoc]
    }
  }

  // Set the superType FK
  if (parentId) {
    await db.updateInCollection('nouns', doc.id, { superType: parentId })
  }

  return result
}
```

- [ ] **Step 4: Register the hook**

In `src/hooks/index.ts`, add at the bottom:

```typescript
import { nounAfterCreate } from './nouns'
COLLECTION_HOOKS['nouns'] = nounAfterCreate
```

- [ ] **Step 5: Run tests**

Run: `npx vitest run src/hooks/nouns.test.ts`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/hooks/nouns.ts src/hooks/nouns.test.ts src/hooks/index.ts
git commit -m "feat: add noun afterCreate hook with subtype parsing"
```

---

### Task 7: Reading hook

**Files:**
- Create: `src/hooks/readings.ts`
- Create: `src/hooks/readings.test.ts`
- Modify: `src/hooks/index.ts` (register hook)

- [ ] **Step 1: Write failing tests**

```typescript
import { describe, it, expect, vi } from 'vitest'
import { readingAfterCreate } from './readings'
import type { HookContext } from './index'

function mockDb(data: Record<string, any[]> = {}) {
  return {
    findInCollection: vi.fn(async (collection: string, where: any) => {
      return { docs: data[collection] || [], totalDocs: 0 }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      return { id: `new-${collection}-${body.name || body.text || 'id'}`, ...body }
    }),
    updateInCollection: vi.fn(async () => ({})),
  }
}

describe('readingAfterCreate', () => {
  it('creates nouns, graph schema, and roles for a simple reading', async () => {
    const db = mockDb()
    const doc = { id: 'r1', text: 'Customer has Name.', domain: 'd1' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }

    const result = await readingAfterCreate(db, doc, ctx)

    // Should have created 2 nouns (Customer as entity, Name as value)
    const nounCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'nouns'
    )
    expect(nounCreates.length).toBe(2)

    // Should have created 1 graph schema
    const schemaCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'graph-schemas'
    )
    expect(schemaCreates.length).toBe(1)
    expect(schemaCreates[0][1].name).toBe('CustomerName')

    // Should have created 2 roles
    const roleCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'roles'
    )
    expect(roleCreates.length).toBe(2)
  })

  it('delegates indented constraint lines via createWithHook', async () => {
    const db = mockDb()
    const doc = {
      id: 'r1',
      text: 'Customer has Name.\n  Each Customer has at most one Name.',
      domain: 'd1',
    }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }

    const result = await readingAfterCreate(db, doc, ctx)

    // Should have created a constraint (delegated)
    const constraintCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'constraints'
    )
    expect(constraintCreates.length).toBeGreaterThanOrEqual(1)
  })

  it('reuses existing nouns (idempotency)', async () => {
    const db = mockDb({
      nouns: [
        { id: 'existing-customer', name: 'Customer' },
        { id: 'existing-name', name: 'Name' },
      ],
    })
    const doc = { id: 'r1', text: 'Customer has Name.', domain: 'd1' }
    const ctx: HookContext = {
      domainId: 'd1',
      allNouns: [
        { name: 'Customer', id: 'existing-customer' },
        { name: 'Name', id: 'existing-name' },
      ],
    }

    await readingAfterCreate(db, doc, ctx)

    // Should NOT have created new nouns
    const nounCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'nouns'
    )
    expect(nounCreates.length).toBe(0)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/hooks/readings.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the reading hook**

```typescript
import { tokenizeReading } from '../claims/tokenize'
import { ensure, createWithHook, refreshNouns, type HookResult, EMPTY_RESULT, type HookContext } from './index'

/**
 * Reading afterCreate hook.
 *
 * 1. Split text into fact type line + indented constraint lines
 * 2. Tokenize reading against known nouns
 * 3. Find-or-create nouns (value type heuristic for "has" objects)
 * 4. Find-or-create graph schema (name = noun concat, title = reading text)
 * 5. Find-or-create roles
 * 6. Delegate constraint lines to createWithHook('constraints', ...)
 */
export async function readingAfterCreate(
  db: any,
  doc: Record<string, any>,
  context: HookContext,
): Promise<HookResult> {
  const rawText = doc.text || ''
  if (!rawText.trim()) return EMPTY_RESULT

  const lines = rawText.split('\n')
  const factLine = lines[0].trim().replace(/\.$/, '')
  const constraintLines = lines.slice(1)
    .filter((l: string) => l.match(/^\s+\S/))
    .map((l: string) => l.trim())

  const result: HookResult = { created: {}, warnings: [] }
  const domainId = context.domainId || doc.domain

  // Refresh nouns to pick up any created in this batch
  let nouns = context.allNouns.length > 0
    ? [...context.allNouns]
    : await refreshNouns(db, domainId)

  // Tokenize to find nouns in the reading
  const tokenized = tokenizeReading(factLine, nouns)
  let nounNames = tokenized.nounRefs.map(r => r.name)

  // If tokenization found fewer than 2 nouns, try extracting PascalCase words
  if (nounNames.length < 2) {
    const pascalWords = factLine.match(/[A-Z][a-zA-Z0-9]*/g) || []
    nounNames = pascalWords
  }

  if (nounNames.length < 2) {
    result.warnings.push(`Reading "${factLine}" has fewer than 2 nouns — skipping`)
    return result
  }

  // Determine predicate for entity/value heuristic
  const predicate = tokenized.predicate || ''
  const isHasPredicate = /^has$/i.test(predicate.trim())

  // Find-or-create nouns
  const nounIds: string[] = []
  for (let i = 0; i < nounNames.length; i++) {
    const name = nounNames[i]
    const existing = nouns.find(n => n.name === name)
    if (existing) {
      nounIds.push(existing.id)
    } else {
      // Heuristic: object of "has" → value type, otherwise entity
      const objectType = (isHasPredicate && i === nounNames.length - 1) ? 'value' : 'entity'
      const { doc: nounDoc } = await ensure(
        db, 'nouns',
        { name: { equals: name }, domain_id: { equals: domainId } },
        { name, objectType, domain: domainId },
      )
      nounIds.push(nounDoc.id)
      nouns.push({ name, id: nounDoc.id })
      result.created['nouns'] = [...(result.created['nouns'] || []), nounDoc]
    }
  }

  // Update context nouns for downstream hooks
  context.allNouns = nouns

  // Find-or-create graph schema
  const schemaName = nounNames.join('')
  const { doc: schema, created: schemaCreated } = await ensure(
    db, 'graph-schemas',
    { name: { equals: schemaName }, domain_id: { equals: domainId } },
    { name: schemaName, title: factLine, domain: domainId },
  )
  if (schemaCreated) {
    result.created['graph-schemas'] = [schema]
  }

  // Link reading to graph schema
  await db.updateInCollection('readings', doc.id, { graphSchema: schema.id })

  // Find-or-create roles
  for (let i = 0; i < nounIds.length; i++) {
    const { doc: role, created: roleCreated } = await ensure(
      db, 'roles',
      {
        reading_id: { equals: doc.id },
        noun_id: { equals: nounIds[i] },
        role_index: { equals: i },
      },
      {
        reading: doc.id,
        noun: nounIds[i],
        graphSchema: schema.id,
        roleIndex: i,
      },
    )
    if (roleCreated) {
      result.created['roles'] = [...(result.created['roles'] || []), role]
    }
  }

  // Delegate constraint lines
  for (const constraintText of constraintLines) {
    try {
      const { hookResult } = await createWithHook(
        db, 'constraints',
        { text: constraintText, domain: domainId },
        context,
      )
      // Merge sub-results
      for (const [key, docs] of Object.entries(hookResult.created)) {
        result.created[key] = [...(result.created[key] || []), ...docs]
      }
      result.warnings.push(...hookResult.warnings)
    } catch (err: any) {
      if (context.batch) {
        context.deferred = context.deferred || []
        context.deferred.push({
          data: { text: constraintText, domain: domainId },
          error: err.message,
        })
      } else {
        result.warnings.push(`Constraint rejected: ${constraintText} — ${err.message}`)
      }
    }
  }

  return result
}
```

- [ ] **Step 4: Register the hook**

In `src/hooks/index.ts`, add:

```typescript
import { readingAfterCreate } from './readings'
COLLECTION_HOOKS['readings'] = readingAfterCreate
```

- [ ] **Step 5: Run tests**

Run: `npx vitest run src/hooks/readings.test.ts`
Expected: All tests pass.

- [ ] **Step 6: Run all tests**

Run: `npx vitest run`
Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/hooks/readings.ts src/hooks/readings.test.ts src/hooks/index.ts
git commit -m "feat: add reading afterCreate hook with noun/schema/role creation"
```

---

## Chunk 4: Constraint Hook + State Machine Hook

### Task 8: Constraint hook

**Files:**
- Create: `src/hooks/constraints.ts`
- Create: `src/hooks/constraints.test.ts`
- Modify: `src/hooks/index.ts` (register hook)

- [ ] **Step 1: Write failing tests**

```typescript
import { describe, it, expect, vi } from 'vitest'
import { constraintAfterCreate } from './constraints'
import type { HookContext } from './index'

function mockDb(data: Record<string, any[]> = {}) {
  return {
    findInCollection: vi.fn(async (collection: string, where: any) => {
      return { docs: data[collection] || [], totalDocs: (data[collection] || []).length }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      return { id: `new-${collection}-id`, ...body }
    }),
    updateInCollection: vi.fn(async (collection: string, id: string, body: any) => {
      return { id, ...body }
    }),
  }
}

describe('constraintAfterCreate', () => {
  const baseContext: HookContext = {
    domainId: 'd1',
    allNouns: [
      { name: 'Customer', id: 'n-customer' },
      { name: 'Name', id: 'n-name' },
    ],
  }

  it('parses natural language UC and creates constraint spans', async () => {
    const db = mockDb({
      readings: [{ id: 'r1', text: 'Customer has Name', domain: 'd1' }],
      roles: [
        { id: 'role-0', readingId: 'r1', nounId: 'n-customer', roleIndex: 0 },
        { id: 'role-1', readingId: 'r1', nounId: 'n-name', roleIndex: 1 },
      ],
    })
    const doc = { id: 'c1', text: 'Each Customer has at most one Name.', domain: 'd1' }

    const result = await constraintAfterCreate(db, doc, baseContext)

    // Should update constraint with parsed kind/modality
    expect(db.updateInCollection).toHaveBeenCalledWith('constraints', 'c1',
      expect.objectContaining({ kind: 'UC', modality: 'Alethic' })
    )
    // Should create constraint spans
    const spanCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'constraint-spans'
    )
    expect(spanCreates.length).toBeGreaterThanOrEqual(1)
  })

  it('rejects constraint when host reading not found (non-batch)', async () => {
    const db = mockDb({ readings: [] })
    const doc = { id: 'c1', text: 'Each Foo has at most one Bar.', domain: 'd1' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }

    const result = await constraintAfterCreate(db, doc, ctx)
    expect(result.warnings.length).toBeGreaterThan(0)
    expect(result.warnings[0]).toContain('host reading not found')
  })

  it('handles shorthand multiplicity format', async () => {
    const db = mockDb({
      readings: [{ id: 'r1', text: 'Customer has Name', domain: 'd1' }],
      roles: [
        { id: 'role-0', readingId: 'r1', nounId: 'n-customer', roleIndex: 0 },
        { id: 'role-1', readingId: 'r1', nounId: 'n-name', roleIndex: 1 },
      ],
    })
    const doc = { id: 'c1', multiplicity: '*:1', reading: 'Customer has Name', domain: 'd1' }

    const result = await constraintAfterCreate(db, doc, baseContext)

    expect(db.updateInCollection).toHaveBeenCalledWith('constraints', 'c1',
      expect.objectContaining({ kind: 'UC' })
    )
  })
})
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/hooks/constraints.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the constraint hook**

```typescript
import { parseConstraintText } from './parse-constraint'
import { parseMultiplicity } from '../claims/constraints'
import { tokenizeReading } from '../claims/tokenize'
import type { HookResult, HookContext } from './index'
import { EMPTY_RESULT } from './index'

/**
 * Constraint afterCreate hook.
 *
 * Accepts two input formats:
 * - Natural language (text field): parse via parseConstraintText()
 * - Shorthand notation (multiplicity field): parse via parseMultiplicity()
 *
 * After parsing, finds the host reading and creates constraint-spans.
 */
export async function constraintAfterCreate(
  db: any,
  doc: Record<string, any>,
  context: HookContext,
): Promise<HookResult> {
  const result: HookResult = { created: {}, warnings: [] }
  const domainId = context.domainId || doc.domain

  let parsedConstraints: Array<{ kind: string; modality: string; deonticOperator?: string }>
  let constraintNouns: string[] = []

  if (doc.text) {
    // Natural language path
    const parsed = parseConstraintText(doc.text)
    if (!parsed) {
      result.warnings.push(`Unrecognized constraint pattern: "${doc.text}"`)
      return result
    }
    parsedConstraints = parsed
    constraintNouns = parsed[0]?.nouns || []
  } else if (doc.multiplicity) {
    // Shorthand notation path
    const defs = parseMultiplicity(doc.multiplicity)
    if (!defs.length) return EMPTY_RESULT
    parsedConstraints = defs.map(d => ({ kind: d.kind, modality: d.modality }))
    // Extract nouns from the reading text if provided
    if (doc.reading) {
      const tokenized = tokenizeReading(doc.reading, context.allNouns)
      constraintNouns = tokenized.nounRefs.map(r => r.name)
    }
  } else {
    return EMPTY_RESULT
  }

  // Find the host reading
  const readingText = doc.reading || ''
  let hostReading: Record<string, any> | null = null
  let hostRoles: Record<string, any>[] = []

  if (readingText) {
    // Direct match by reading text
    const readings = await db.findInCollection('readings', {
      text: { equals: readingText },
      domain_id: { equals: domainId },
    }, { limit: 1 })
    if (readings.docs.length > 0) hostReading = readings.docs[0]
  }

  if (!hostReading && constraintNouns.length >= 2) {
    // Match by noun set: find readings containing the same nouns
    const allReadings = await db.findInCollection('readings', {
      domain_id: { equals: domainId },
    }, { limit: 0 })

    for (const reading of allReadings.docs) {
      const tokenized = tokenizeReading(
        (reading.text || '').split('\n')[0].replace(/\.$/, ''),
        context.allNouns,
      )
      const readingNouns = tokenized.nounRefs.map(r => r.name)
      if (readingNouns.length === constraintNouns.length &&
          readingNouns.every((n, i) => n === constraintNouns[i])) {
        hostReading = reading
        break
      }
    }
  }

  if (!hostReading) {
    if (context.batch) {
      // In batch mode, defer rather than reject
      context.deferred = context.deferred || []
      context.deferred.push({
        data: { ...doc },
        error: `host reading not found for constraint: "${doc.text || doc.multiplicity}"`,
      })
      return result
    }
    result.warnings.push(
      `Constraint rejected: host reading not found for "${doc.text || doc.multiplicity}"`
    )
    return result
  }

  // Fetch roles for the host reading
  const rolesResult = await db.findInCollection('roles', {
    reading_id: { equals: hostReading.id },
  }, { limit: 0 })
  hostRoles = rolesResult.docs.sort((a: any, b: any) => a.roleIndex - b.roleIndex)

  if (hostRoles.length === 0) {
    result.warnings.push(`No roles found for reading "${hostReading.text}"`)
    return result
  }

  // Create constraint records and spans for each parsed constraint
  for (const parsed of parsedConstraints) {
    // Update the already-created constraint doc with parsed kind/modality
    await db.updateInCollection('constraints', doc.id, {
      kind: parsed.kind,
      modality: parsed.modality,
    })

    // Determine which roles to span
    let roleIds: string[]
    if (parsed.kind === 'RC') {
      // Ring constraint spans the first role (self-referential)
      roleIds = hostRoles.length > 0 ? [hostRoles[0].id] : []
    } else if (constraintNouns.length === 2 && hostRoles.length >= 2) {
      // Binary: "Each X ..." constrains role 0 (the X side)
      roleIds = [hostRoles[0].id]
    } else {
      // Spanning or ternary: all roles
      roleIds = hostRoles.map((r: any) => r.id)
    }

    for (const roleId of roleIds) {
      const span = await db.createInCollection('constraint-spans', {
        constraint: doc.id,
        role: roleId,
      })
      result.created['constraint-spans'] = [
        ...(result.created['constraint-spans'] || []),
        span,
      ]
    }
  }

  return result
}
```

- [ ] **Step 4: Register the hook**

In `src/hooks/index.ts`, add:

```typescript
import { constraintAfterCreate } from './constraints'
COLLECTION_HOOKS['constraints'] = constraintAfterCreate
```

- [ ] **Step 5: Run tests**

Run: `npx vitest run src/hooks/constraints.test.ts`
Expected: All tests pass.

- [ ] **Step 6: Run all tests**

Run: `npx vitest run`
Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/hooks/constraints.ts src/hooks/constraints.test.ts src/hooks/index.ts
git commit -m "feat: add constraint afterCreate hook with NL parsing and host reading resolution"
```

---

### Task 9: State machine definition hook

**Files:**
- Create: `src/hooks/state-machines.ts`
- Create: `src/hooks/state-machines.test.ts`
- Modify: `src/hooks/index.ts` (register hook)

- [ ] **Step 1: Write failing test**

```typescript
import { describe, it, expect, vi } from 'vitest'
import { smDefinitionAfterCreate } from './state-machines'
import type { HookContext } from './index'

function mockDb(data: Record<string, any[]> = {}) {
  return {
    findInCollection: vi.fn(async (collection: string, where: any) => {
      return { docs: data[collection] || [], totalDocs: 0 }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      return { id: `new-${collection}-${body.name || body.title || 'id'}`, ...body }
    }),
  }
}

describe('smDefinitionAfterCreate', () => {
  it('creates statuses, event types, and transitions from transition data', async () => {
    const db = mockDb({
      nouns: [{ id: 'noun-sr', name: 'SupportRequest' }],
    })
    const doc = {
      id: 'smd1',
      title: 'SupportRequest',
      domain: 'd1',
      transitions: [
        { from: 'Received', to: 'Triaging', event: 'acknowledge' },
        { from: 'Triaging', to: 'Investigating', event: 'assign' },
      ],
    }
    const ctx: HookContext = {
      domainId: 'd1',
      allNouns: [{ name: 'SupportRequest', id: 'noun-sr' }],
    }

    const result = await smDefinitionAfterCreate(db, doc, ctx)

    // Should create 3 statuses (Received, Triaging, Investigating)
    const statusCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'statuses'
    )
    expect(statusCreates.length).toBe(3)

    // Should create 2 event types (acknowledge, assign)
    const eventCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'event-types'
    )
    expect(eventCreates.length).toBe(2)

    // Should create 2 transitions
    const transitionCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'transitions'
    )
    expect(transitionCreates.length).toBe(2)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/hooks/state-machines.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the state machine definition hook**

```typescript
import { ensure, type HookResult, EMPTY_RESULT, type HookContext } from './index'

/**
 * StateMachineDefinition afterCreate hook.
 *
 * Creates statuses, event types, and transitions from transition data
 * provided in the doc.
 */
export async function smDefinitionAfterCreate(
  db: any,
  doc: Record<string, any>,
  context: HookContext,
): Promise<HookResult> {
  const transitions = doc.transitions as Array<{
    from: string; to: string; event: string; guard?: string
  }> | undefined

  if (!transitions || transitions.length === 0) return EMPTY_RESULT

  const result: HookResult = { created: {}, warnings: [] }
  const domainId = context.domainId || doc.domain
  const definitionId = doc.id

  // Find-or-create the target noun
  const nounName = doc.title || doc.name
  if (nounName) {
    await ensure(
      db, 'nouns',
      { name: { equals: nounName }, domain_id: { equals: domainId } },
      { name: nounName, objectType: 'entity', domain: domainId },
    )
  }

  // Collect unique status names and event names
  const statusNames = new Set<string>()
  const eventNames = new Set<string>()
  for (const t of transitions) {
    statusNames.add(t.from)
    statusNames.add(t.to)
    eventNames.add(t.event)
  }

  // Find-or-create statuses
  const statusMap = new Map<string, string>() // name → id
  for (const name of statusNames) {
    const { doc: status, created } = await ensure(
      db, 'statuses',
      {
        name: { equals: name },
        state_machine_definition_id: { equals: definitionId },
      },
      {
        name,
        stateMachineDefinition: definitionId,
        domain: domainId,
      },
    )
    statusMap.set(name, status.id)
    if (created) {
      result.created['statuses'] = [...(result.created['statuses'] || []), status]
    }
  }

  // Find-or-create event types
  const eventMap = new Map<string, string>() // name → id
  for (const name of eventNames) {
    const { doc: eventType, created } = await ensure(
      db, 'event-types',
      { name: { equals: name }, domain_id: { equals: domainId } },
      { name, domain: domainId },
    )
    eventMap.set(name, eventType.id)
    if (created) {
      result.created['event-types'] = [...(result.created['event-types'] || []), eventType]
    }
  }

  // Create transitions
  for (const t of transitions) {
    const fromId = statusMap.get(t.from)!
    const toId = statusMap.get(t.to)!
    const eventId = eventMap.get(t.event)!

    const transition = await db.createInCollection('transitions', {
      fromStatus: fromId,
      toStatus: toId,
      eventType: eventId,
      domain: domainId,
    })
    result.created['transitions'] = [...(result.created['transitions'] || []), transition]

    // Create guard if provided
    if (t.guard) {
      const guard = await db.createInCollection('guards', {
        name: t.guard,
        transition: transition.id,
        domain: domainId,
      })
      result.created['guards'] = [...(result.created['guards'] || []), guard]
    }
  }

  return result
}
```

- [ ] **Step 4: Register the hook**

In `src/hooks/index.ts`, add:

```typescript
import { smDefinitionAfterCreate } from './state-machines'
COLLECTION_HOOKS['state-machine-definitions'] = smDefinitionAfterCreate
```

- [ ] **Step 5: Run tests**

Run: `npx vitest run src/hooks/state-machines.test.ts`
Expected: All tests pass.

- [ ] **Step 6: Run all tests**

Run: `npx vitest run`
Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/hooks/state-machines.ts src/hooks/state-machines.test.ts src/hooks/index.ts
git commit -m "feat: add state machine definition afterCreate hook"
```

---

## Chunk 5: Integration + ingestClaims Simplification

### Task 10: End-to-end integration test

**Files:**
- Create: `src/hooks/integration.test.ts`

- [ ] **Step 1: Write integration test for Reading with constraints**

```typescript
import { describe, it, expect, vi } from 'vitest'
import { createWithHook, type HookContext } from './index'

// Import to ensure hooks are registered
import './nouns'
import './readings'
import './constraints'
import './state-machines'

/**
 * Integration test using a mock DB that accumulates state.
 * Verifies that creating a Reading with indented constraints
 * triggers the full hook chain.
 */
function statefulMockDb() {
  const store: Record<string, Record<string, any>[]> = {
    nouns: [],
    'graph-schemas': [],
    readings: [],
    roles: [],
    constraints: [],
    'constraint-spans': [],
  }
  let counter = 0

  return {
    store,
    findInCollection: vi.fn(async (collection: string, where: any, opts?: any) => {
      const docs = store[collection] || []
      // Simple filtering by where clauses
      const filtered = docs.filter(doc => {
        for (const [field, condition] of Object.entries(where)) {
          const cond = condition as any
          if (cond.equals !== undefined) {
            // Map Payload field names to possible stored field names
            const val = doc[field] ?? doc[field.replace('_id', '')] ?? doc[field.replace(/_/g, '')]
            if (val !== cond.equals) return false
          }
        }
        return true
      })
      return { docs: filtered, totalDocs: filtered.length }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      const id = `${collection}-${++counter}`
      const doc = { id, ...body }
      if (!store[collection]) store[collection] = []
      store[collection].push(doc)
      return doc
    }),
    updateInCollection: vi.fn(async (collection: string, id: string, body: any) => {
      const coll = store[collection] || []
      const idx = coll.findIndex(d => d.id === id)
      if (idx >= 0) Object.assign(coll[idx], body)
      return coll[idx] || { id, ...body }
    }),
  }
}

describe('Hook composition integration', () => {
  it('Reading with indented constraint creates full object graph', async () => {
    const db = statefulMockDb()
    const context: HookContext = { domainId: 'd1', allNouns: [] }

    // Simulate what the POST handler does
    const readingData = {
      text: 'Customer has Name.\n  Each Customer has at most one Name.',
      domain: 'd1',
    }

    const { doc, hookResult } = await createWithHook(db, 'readings', readingData, context)

    // Reading was created
    expect(doc.text).toContain('Customer has Name')

    // Nouns were created
    expect(db.store['nouns'].length).toBe(2)
    const nounNames = db.store['nouns'].map(n => n.name).sort()
    expect(nounNames).toEqual(['Customer', 'Name'])

    // Graph schema was created
    expect(db.store['graph-schemas'].length).toBe(1)
    expect(db.store['graph-schemas'][0].name).toBe('CustomerName')

    // Roles were created
    expect(db.store['roles'].length).toBe(2)

    // Constraint was created
    expect(db.store['constraints'].length).toBeGreaterThanOrEqual(1)
  })
})
```

- [ ] **Step 2: Run integration test**

Run: `npx vitest run src/hooks/integration.test.ts`
Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/hooks/integration.test.ts
git commit -m "test: add integration test for hook composition chain"
```

---

### Task 11: Run full test suite and verify

- [ ] **Step 1: Run all tests**

Run: `npx vitest run`
Expected: All tests pass including existing tests and all new hook tests.

- [ ] **Step 2: Final commit if any adjustments were needed**

```bash
git add -A
git commit -m "fix: adjustments from full test suite run"
```
