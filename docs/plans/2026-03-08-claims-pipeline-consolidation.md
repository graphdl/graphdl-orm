# Claims Pipeline Consolidation

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Consolidate 4 duplicate implementations of noun tokenization, constraint creation, and reading ingestion into a single `src/claims/` module. Kill the seed route. Make the claims endpoint the single open-source entry point for deterministic claim ingestion.

**Architecture:** A `src/claims/` module exports core functions that both the HTTP endpoint (`/api/claims`) and collection hooks (`Readings.afterChange`, `GraphSchemas.roleRelationship`) call. Parsing stays in `src/parse/`, markdown parsing stays in `src/seed/parser.ts`. The apis worker calls the claims endpoint via HTTP.

**Tech Stack:** Payload CMS 3.x local API, TypeScript, Vitest

---

## Current Duplication Map

| Logic | Location 1 | Location 2 | Location 3 | Location 4 |
|-------|-----------|-----------|-----------|-----------|
| Noun tokenization (longest-first regex) | `parse/forml2.ts:buildNounRegex` | `Readings.ts` afterChange hook (lines 52-59) | `handler.ts:applySubsetConstraint` (line 393) | |
| Constraint creation (multiplicity → UC/MC) | `handler.ts:applyConstraints` | `GraphSchemas.ts` roleRelationship hook | `route.ts:seedFromClaims` | `unified.ts` |
| Subtype application | `handler.ts:applySubtype` | `unified.ts` (lines 144-155) | `route.ts:seedFromClaims` (lines 92-105) | |
| Instance fact parsing | `parse/forml2.ts:parseReading` | `handler.ts:parseInstanceFact` | | |

## Target Architecture

```
src/claims/
  index.ts          — exports all public functions + ExtractedClaims types
  tokenize.ts       — tokenizeReading(text, nouns) → noun refs in order
  constraints.ts    — applyMultiplicity(payload, { multiplicity, roles, domainId })
  ingest.ts         — ingestReading(), ingestClaims()

src/app/(api)/claims/route.ts  — HTTP endpoint (replaces /seed)

src/collections/Readings.ts    — afterChange calls claims.ingestReading()
src/collections/GraphSchemas.ts — roleRelationship calls claims.applyMultiplicity()
```

---

### Task 1: Create `src/claims/tokenize.ts` — Single Noun Tokenizer

**Files:**
- Create: `src/claims/tokenize.ts`
- Test: `src/claims/tokenize.test.ts`

**Step 1: Write the failing test**

```typescript
import { describe, it, expect } from 'vitest'
import { tokenizeReading } from './tokenize'

describe('tokenizeReading', () => {
  it('should find nouns in reading text in order', () => {
    const nouns = [
      { name: 'Customer', id: 'c1' },
      { name: 'SupportRequest', id: 'sr1' },
    ]
    const result = tokenizeReading('Customer submits SupportRequest', nouns)
    expect(result.nounRefs).toEqual([
      { name: 'Customer', id: 'c1', index: 0 },
      { name: 'SupportRequest', id: 'sr1', index: 1 },
    ])
    expect(result.predicate).toBe('submits')
  })

  it('should match longest noun first', () => {
    const nouns = [
      { name: 'Request', id: 'r1' },
      { name: 'SupportRequest', id: 'sr1' },
      { name: 'Customer', id: 'c1' },
    ]
    const result = tokenizeReading('Customer submits SupportRequest', nouns)
    expect(result.nounRefs).toHaveLength(2)
    expect(result.nounRefs[1].name).toBe('SupportRequest')
  })

  it('should return empty for no matches', () => {
    const result = tokenizeReading('hello world', [])
    expect(result.nounRefs).toEqual([])
  })
})
```

**Step 2: Run test to verify it fails**

Run: `npx vitest run src/claims/tokenize.test.ts`
Expected: FAIL — module not found

**Step 3: Write minimal implementation**

```typescript
export interface NounRef {
  name: string
  id: string
  index: number
}

export interface TokenizeResult {
  nounRefs: NounRef[]
  predicate: string
}

/**
 * Tokenize a reading text against a list of known nouns.
 * Returns noun references in the order they appear, plus the predicate
 * (text between first and second noun).
 *
 * Uses longest-first matching to avoid partial matches
 * (e.g., "SupportRequest" before "Request").
 */
export function tokenizeReading(
  text: string,
  nouns: Array<{ name: string; id: string }>,
): TokenizeResult {
  if (!nouns.length) return { nounRefs: [], predicate: '' }

  const sorted = [...nouns].sort((a, b) => b.name.length - a.name.length)
  const regex = new RegExp('\\b(' + sorted.map((n) => n.name).join('|') + ')\\b')

  const nounRefs: NounRef[] = []
  let index = 0
  text.split(regex).forEach((token) => {
    const noun = nouns.find((n) => n.name === token)
    if (noun) {
      nounRefs.push({ name: noun.name, id: noun.id, index: index++ })
    }
  })

  // Extract predicate: text between first and second noun
  let predicate = ''
  if (nounRefs.length >= 2) {
    const first = nounRefs[0].name
    const second = nounRefs[1].name
    const between = text.split(first)[1]?.split(second)[0]?.trim()
    if (between) predicate = between
  }

  return { nounRefs, predicate }
}
```

**Step 4: Run test to verify it passes**

Run: `npx vitest run src/claims/tokenize.test.ts`
Expected: PASS

**Step 5: Commit**

```bash
git add src/claims/tokenize.ts src/claims/tokenize.test.ts
git commit -m "feat(claims): add tokenizeReading — single noun tokenizer"
```

---

### Task 2: Create `src/claims/constraints.ts` — Single Constraint Creator

**Files:**
- Create: `src/claims/constraints.ts`
- Test: `src/claims/constraints.test.ts`
- Reference: `src/seed/handler.ts` lines 264-410 (`applyConstraints`, `parseConstraintSpec`)

**Step 1: Write the failing test**

```typescript
import { describe, it, expect } from 'vitest'
import { parseMultiplicity } from './constraints'

describe('parseMultiplicity', () => {
  it('should parse *:1 as UC on role 0', () => {
    const result = parseMultiplicity('*:1')
    expect(result).toEqual([{ kind: 'UC', modality: 'Alethic', roles: [0] }])
  })

  it('should parse 1:* as UC on role 1', () => {
    const result = parseMultiplicity('1:*')
    expect(result).toEqual([{ kind: 'UC', modality: 'Alethic', roles: [1] }])
  })

  it('should parse 1:1 as two UCs', () => {
    const result = parseMultiplicity('1:1')
    expect(result).toHaveLength(2)
  })

  it('should parse *:* as UC spanning both roles', () => {
    const result = parseMultiplicity('*:*')
    expect(result).toEqual([{ kind: 'UC', modality: 'Alethic', roles: [0, 1] }])
  })

  it('should parse *:1 MC as UC + MC', () => {
    const result = parseMultiplicity('*:1 MC')
    expect(result).toContainEqual({ kind: 'UC', modality: 'Alethic', roles: [0] })
    expect(result).toContainEqual({ kind: 'MC', modality: 'Alethic', roles: [1] })
  })
})
```

**Step 2: Write implementation**

Extract `parseConstraintSpec()` and constraint creation from `handler.ts` into a pure function:

- `parseMultiplicity(spec: string): ConstraintDef[]` — pure function, no Payload dependency
- `applyConstraints(payload, { constraints, roleIds, domainId })` — creates constraint + constraint-span records

The `applyConstraints` function replaces the 4 duplicate implementations. It takes an array of `ConstraintDef` (from `parseMultiplicity` or from structured LLM claims) and creates the Payload records.

**Step 3: Commit**

```bash
git add src/claims/constraints.ts src/claims/constraints.test.ts
git commit -m "feat(claims): add parseMultiplicity and applyConstraints"
```

---

### Task 3: Create `src/claims/ingest.ts` — Core Ingestion Functions

**Files:**
- Create: `src/claims/ingest.ts`
- Test: `src/claims/ingest.test.ts` (integration test with Payload)
- Reference: `src/seed/unified.ts`, `src/app/seed/route.ts:seedFromClaims`, `src/seed/handler.ts:seedReadings`

**Core exports:**

```typescript
/** Ingest a single reading — creates graph schema if needed, creates reading
 *  (hook auto-creates roles), applies constraints from multiplicity.
 *  Used by: Readings.afterChange hook, admin UI, single reading creation. */
export async function ingestReading(
  payload: Payload,
  opts: {
    text: string
    graphSchemaId?: string  // if omitted, auto-creates from noun names
    domainId?: string
    multiplicity?: string   // e.g. "*:1"
  },
): Promise<IngestReadingResult>

/** Ingest bulk structured claims — nouns, readings, constraints, subtypes,
 *  transitions, instance facts. The open-source deterministic entry point.
 *  Used by: /api/claims endpoint. */
export async function ingestClaims(
  payload: Payload,
  opts: {
    claims: ExtractedClaims
    domainId: string
  },
): Promise<IngestClaimsResult>
```

**Implementation approach:**

1. `ingestReading` calls `tokenizeReading` to find nouns, creates graph schema if needed, creates the reading (which triggers the afterChange hook for role creation), then calls `applyConstraints` if multiplicity is provided.

2. `ingestClaims` orchestrates: create nouns → apply subtypes → call `ingestReading` for each reading → apply additional constraints → seed transitions → seed instance facts.

**Key decision:** The Readings afterChange hook currently does its own tokenization. After this refactor, the hook should call `tokenizeReading` from the claims module instead. But `ingestReading` also needs to create roles — so there's a question of whether the hook or `ingestReading` owns role creation.

**Resolution:** The hook stays as the role creator (it fires on ANY reading creation, including admin UI). But it delegates tokenization to `tokenizeReading()`. `ingestReading` does NOT create roles directly — it creates the reading, which triggers the hook. This preserves the existing behavior for admin UI users.

**Step 1: Write integration test**

Use the existing test pattern: `initPayload()`, drop database, create domain + nouns, call `ingestReading`, assert roles were created.

**Step 2: Implement `ingestReading` and `ingestClaims`**

Pull logic from:
- `unified.ts:seedReadingsFromText` → two-pass noun discovery pattern
- `route.ts:seedFromClaims` → structured claims → database writes
- `handler.ts:seedReadings` → reading + constraint creation flow

**Step 3: Commit**

```bash
git add src/claims/ingest.ts src/claims/ingest.test.ts
git commit -m "feat(claims): add ingestReading and ingestClaims"
```

---

### Task 4: Create `src/claims/index.ts` — Barrel Export + ExtractedClaims Types

**Files:**
- Create: `src/claims/index.ts`

**Step 1: Move `ExtractedClaims` types from apis**

The canonical types for structured claims move here (open source). These are currently defined in `apis/graphdl/extract-claims.ts`.

```typescript
// src/claims/index.ts
export type { TokenizeResult, NounRef } from './tokenize'
export { tokenizeReading } from './tokenize'
export { parseMultiplicity, applyConstraints } from './constraints'
export { ingestReading, ingestClaims } from './ingest'

// Canonical types — also used by apis worker
export interface ExtractedClaims { ... }
export interface ExtractedNoun { ... }
export interface ExtractedReading { ... }
export interface ExtractedConstraint { ... }
export interface ExtractedSubtype { ... }
export interface ExtractedTransition { ... }
export interface ExtractedFact { ... }
```

**Step 2: Commit**

```bash
git add src/claims/index.ts
git commit -m "feat(claims): barrel export + ExtractedClaims types"
```

---

### Task 5: Refactor `Readings.afterChange` Hook

**Files:**
- Modify: `src/collections/Readings.ts`

**Step 1: Replace inline noun tokenization with `tokenizeReading()`**

Before (lines 43-66):
```typescript
// Fetch all nouns, build regex, split, find matches — 24 lines of inline logic
```

After:
```typescript
import { tokenizeReading } from '../claims'

// Fetch nouns (still needed for the noun list)
const nouns = await payload.find({ collection: 'nouns', pagination: false, req })
const nounList = nouns.docs.map((n: any) => ({ name: n.name, id: n.id }))

// Also include objectified graph schemas as potential nouns
const graphSchemas = await payload.find({ collection: 'graph-schemas', pagination: false, depth: 3, req })
const graphNouns = graphSchemas.docs
  .filter((g: any) => g.title === g.name)
  .map((g: any) => ({ name: g.name, id: g.id, collection: 'graph-schemas' }))

const allEntities = [...graphNouns, ...nounList]
const { nounRefs } = tokenizeReading(doc.text, allEntities)
```

Role creation loop stays in the hook (it's the canonical role creator). Only the tokenization is delegated.

**Step 2: Run existing tests**

Run: `npx vitest run test/collections/`
Expected: All existing tests still pass

**Step 3: Commit**

```bash
git add src/collections/Readings.ts
git commit -m "refactor(readings): delegate noun tokenization to claims module"
```

---

### Task 6: Refactor `GraphSchemas.roleRelationship` Hook

**Files:**
- Modify: `src/collections/GraphSchemas.ts`

**Step 1: Replace inline constraint creation with `applyConstraints()`**

The `roleRelationship` beforeChange hook currently has ~60 lines of constraint creation logic. Replace with:

```typescript
import { parseMultiplicity, applyConstraints } from '../claims'

// Map UI enum to multiplicity notation
const enumToMult: Record<string, string> = {
  'many-to-one': '*:1',
  'one-to-many': '1:*',
  'many-to-many': '*:*',
  'one-to-one': '1:1',
}
const mult = enumToMult[data.roleRelationship]
if (mult) {
  const constraintDefs = parseMultiplicity(mult)
  const roleIds = roles.docs.map((r: any) => r.id)
  await applyConstraints(payload, { constraints: constraintDefs, roleIds, domainId })
}
```

**Step 2: Run existing tests**

Run: `npx vitest run test/collections/graph-schemas.test.ts`
Expected: PASS

**Step 3: Commit**

```bash
git add src/collections/GraphSchemas.ts
git commit -m "refactor(graph-schemas): delegate constraint creation to claims module"
```

---

### Task 7: Create `/api/claims` Route (Replaces `/seed`)

**Files:**
- Create: `src/app/(api)/claims/route.ts`
- Reference: `src/app/seed/route.ts`

**Step 1: Create new route handler**

```typescript
import { getPayload } from 'payload'
import config from '@payload-config'
import { ingestClaims } from '../../../claims'

export async function POST(req: Request) {
  const payload = await getPayload({ config })
  const body = await req.json()
  const { claims, domainId } = body

  if (!claims || !domainId) {
    return Response.json({ error: 'claims and domainId required' }, { status: 400 })
  }

  const result = await ingestClaims(payload, { claims, domainId })
  return Response.json(result)
}
```

Keep GET (database counts) if useful. Drop DELETE (dangerous, should be admin-only if at all).

**Step 2: Verify endpoint works**

Run dev server, POST structured claims, verify nouns + readings + constraints created.

**Step 3: Commit**

```bash
git add src/app/\(api\)/claims/route.ts
git commit -m "feat: add /api/claims endpoint (replaces /seed)"
```

---

### Task 8: Delete Seed Route + Consolidate Dead Code

**Files:**
- Delete: `src/app/seed/route.ts`
- Delete: `src/seed/unified.ts`
- Delete: `src/seed/handler.ts` (after extracting any remaining unique logic)
- Keep: `src/seed/parser.ts` (markdown parsing — different concern)
- Keep: `src/seed/deontic.ts` (if still needed by claims module)
- Keep: `src/parse/` (text parsing — different concern, used by legacy paths)
- Update: move/update tests as needed

**Step 1: Audit remaining unique logic in handler.ts**

Before deleting, check for logic not yet moved to claims module:
- `seedStateMachine()` — state machine seeding, may stay as a separate module or move to claims
- `seedInstanceFacts()` — instance fact creation, should be in claims module
- `wireVerbsAndFunctions()` — verb/function wiring for state machines
- `ensureDomain()` — idempotent domain creation helper (move to claims or shared)
- `applySubsetConstraint()` — subset constraints (move to claims/constraints.ts)

**Step 2: Move remaining logic, then delete**

**Step 3: Update test imports**

Tests referencing deleted modules need to import from `src/claims/` instead.

**Step 4: Commit**

```bash
git rm src/app/seed/route.ts src/seed/unified.ts src/seed/handler.ts
git add -A
git commit -m "refactor: remove seed route, handler, unified — logic consolidated in claims module"
```

---

### Task 9: Update apis Worker

**Files (in `C:\Users\lippe\Repos\apis`):**
- Delete: `graphdl/seed-proxy.ts`
- Modify: `graphdl/bootstrap.ts` — call `/api/claims` instead of `/seed`
- Modify: `graphdl/extract-claims.ts` — call `/api/claims` instead of `/seed`, fold in semantic extraction
- Delete or modify: `graphdl/extract-semantic.ts` — fold into extract-claims
- Modify: `index.ts` — remove `/graphdl/seed` route, remove seed-proxy import

**Step 1: Update bootstrap.ts**

Change `fetch(\`${env.GRAPHDL_URL}/seed\`, ...)` to `fetch(\`${env.GRAPHDL_URL}/api/claims\`, ...)` with the new payload shape.

**Step 2: Update extract-claims.ts**

- Change seed call to use `/api/claims`
- Fold `extractSemantic` logic into this file (single LLM extraction + violation checking)

**Step 3: Remove seed-proxy**

```bash
git rm graphdl/seed-proxy.ts
```

**Step 4: Update index.ts routes**

Remove:
```typescript
.all('/graphdl/seed', assertAuthenticated, assertPaidSubscription('Starter'), seedProxy)
```

**Step 5: Commit**

```bash
git add -A
git commit -m "refactor(apis): use /api/claims endpoint, remove seed proxy"
```

---

### Task 10: Update ExtractedClaims Import in apis

**Files:**
- Modify: `apis/graphdl/extract-claims.ts`

The `ExtractedClaims` interface now lives in graphdl-orm's claims module. Since apis calls graphdl-orm via HTTP (not import), apis keeps its own copy of the types. But they should stay in sync.

Options:
1. apis mirrors the types (current approach, just move within apis)
2. graphdl-orm publishes types as an npm package (future)

For now, keep the types in apis but add a comment referencing the canonical source:

```typescript
/** Mirrors ExtractedClaims from graphdl-orm/src/claims/index.ts */
```

**Commit:**

```bash
git add graphdl/extract-claims.ts
git commit -m "docs: reference canonical ExtractedClaims types in graphdl-orm"
```

---

## Dependency Order

```
Task 1 (tokenize)        — no deps
Task 2 (constraints)     — no deps
Task 3 (ingest)          — depends on 1, 2
Task 4 (barrel + types)  — depends on 1, 2, 3
Task 5 (Readings hook)   — depends on 1
Task 6 (GraphSchemas hook) — depends on 2
Task 7 (claims route)    — depends on 3, 4
Task 8 (delete seed)     — depends on 7 (new route must exist first)
Task 9 (apis update)     — depends on 8 (old route removed)
Task 10 (types sync)     — depends on 4, 9
```

Tasks 1+2 can run in parallel. Tasks 5+6 can run in parallel after their deps. Tasks 9+10 are apis-side and should be a separate PR.
