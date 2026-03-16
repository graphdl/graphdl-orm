# Combined Extraction Endpoints (graphdl-orm) Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `/parse` and `/verify` endpoints to graphdl-orm — deterministic, LLM-free entry points for FORML2 parsing and noun-matching verification.

**Architecture:** Two new handler files (`src/api/parse.ts`, `src/api/verify.ts`) registered at root level in `router.ts`. Both are pure-function pipelines — no DB writes, no hooks. `/parse` reuses `tokenizeReading()` and `parseConstraintText()` as pure functions. `/verify` loads domain data read-only and tokenizes prose.

**Tech Stack:** TypeScript, itty-router, Cloudflare Workers + Durable Object, vitest

**Spec:** `docs/specs/2026-03-12-combined-extraction-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `src/api/parse.ts` | Create | `/parse` handler — pure-function FORML2 text → `ExtractedClaims` |
| `src/api/parse.test.ts` | Create | Unit tests for `/parse` |
| `src/api/verify.ts` | Create | `/verify` handler — prose + domain → `{ matches, unmatchedConstraints }` |
| `src/api/verify.test.ts` | Create | Unit tests for `/verify` |
| `src/api/router.ts` | Modify (lines 219-223) | Register `/parse` and `/verify` routes before the `*` fallback |
| `src/claims/ingest.ts` | Modify (line 30) | Add `'RC'` to constraint kind type union |

---

## Chunk 0: Update `ExtractedClaims` interface

### Task 0: Add `'RC'` to `ExtractedClaims` constraint kind

**Files:**
- Modify: `src/claims/ingest.ts:30`

The `ExtractedClaims` interface currently defines `kind: 'UC' | 'MC'` but `parseConstraintText()` can produce `'RC'` (ring constraints). The spec includes `'RC'` in its definition. Update the canonical interface.

- [ ] **Step 1: Update the type union**

In `src/claims/ingest.ts`, change line 30 from:
```typescript
    kind: 'UC' | 'MC'
```
to:
```typescript
    kind: 'UC' | 'MC' | 'RC'
```

- [ ] **Step 2: Commit**

```bash
git add src/claims/ingest.ts
git commit -m "fix: add RC to ExtractedClaims constraint kind union"
```

---

## Chunk 1: `/parse` endpoint

### Task 1: Parse handler — core parsing logic

**Files:**
- Create: `src/api/parse.ts`
- Test: `src/api/parse.test.ts`

The `/parse` handler is a pure-function pipeline. It does NOT use `createWithHook()`, hooks, or write to the database. It loads existing nouns for tokenization context (read-only), then parses FORML2 text into structured `ExtractedClaims`.

#### Algorithm

1. Load existing nouns for the domain via `db.findInCollection('nouns', { domain_id: { equals: domainId } })` (read-only context for tokenization).
2. Split `text` on blank lines into blocks. Each block = first line (reading) + subsequent indented lines (constraints).
3. For each block:
   a. Strip trailing period from the fact line.
   b. Tokenize against known nouns via `tokenizeReading()`.
   c. If fewer than 2 nouns found, try PascalCase regex fallback: `/[A-Z][a-zA-Z0-9]*/g`.
   d. If still fewer than 2 nouns → add warning, skip this block.
   e. Determine predicate from text between first two nouns.
   f. Entity/value heuristic: object of "has" → `value`, otherwise `entity`.
   g. Accumulate nouns (deduplicating by name) with their object types.
   h. Accumulate reading `{ text, nouns, predicate }`.
   i. Parse each indented constraint via `parseConstraintText()`.
   j. For each parsed constraint, map to `ExtractedClaims.constraints` format using role indices (position of constraint nouns within the reading's noun list).
   k. Unrecognized constraint → warning, skip.
4. Detect subtype declarations via regex: `/^(\w+) is a subtype of (\w+)/i` → add to subtypes array.
5. Handle deferred constraints: if a constraint references nouns not seen yet, defer it. After all blocks, retry deferred constraints against the full noun set. Still-unresolved → warning.
6. Return `{ ...ExtractedClaims, warnings }` with `transitions: []` and `facts: []` (always empty).

- [ ] **Step 1: Write the failing test for basic FORML2 parsing**

Create `src/api/parse.test.ts` with the first test: a single reading with one constraint.

```typescript
import { describe, it, expect } from 'vitest'
import { parseFORML2 } from './parse'

describe('parseFORML2', () => {
  it('parses a single reading with UC constraint', () => {
    const text = `Customer has Name.
  Each Customer has at most one Name.`

    const result = parseFORML2(text, [])

    // Nouns
    expect(result.nouns).toHaveLength(2)
    expect(result.nouns.find(n => n.name === 'Customer')).toMatchObject({
      name: 'Customer',
      objectType: 'entity',
    })
    expect(result.nouns.find(n => n.name === 'Name')).toMatchObject({
      name: 'Name',
      objectType: 'value', // object of "has" → value type
    })

    // Readings
    expect(result.readings).toHaveLength(1)
    expect(result.readings[0]).toMatchObject({
      text: 'Customer has Name',
      nouns: ['Customer', 'Name'],
      predicate: 'has',
    })

    // Constraints
    expect(result.constraints).toHaveLength(1)
    expect(result.constraints[0]).toMatchObject({
      kind: 'UC',
      modality: 'Alethic',
      reading: 'Customer has Name',
      roles: [0],
    })

    // Always empty
    expect(result.transitions).toEqual([])
    expect(result.facts).toEqual([])
    expect(result.warnings).toEqual([])
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/parse.test.ts`
Expected: FAIL — `parseFORML2` does not exist

- [ ] **Step 3: Write the `parseFORML2` pure function**

Create `src/api/parse.ts` with the core parsing function (exported separately for testability) and the HTTP handler.

```typescript
import { json, error } from 'itty-router'
import type { Env } from '../types'
import type { ExtractedClaims } from '../claims/ingest'
import { tokenizeReading } from '../claims/tokenize'
import { parseConstraintText } from '../hooks/parse-constraint'

interface ParseResult extends ExtractedClaims {
  warnings: string[]
}

/**
 * Pure-function FORML2 parser.
 *
 * Parses multi-line FORML2 text into structured ExtractedClaims.
 * No DB writes, no hooks — read-only.
 *
 * @param text - Multi-line FORML2 text
 * @param existingNouns - Known nouns for tokenization context (from DB, read-only)
 */
export function parseFORML2(
  text: string,
  existingNouns: Array<{ name: string; id: string; objectType?: 'entity' | 'value' }>,
): ParseResult {
  const warnings: string[] = []
  const nounMap = new Map<string, { name: string; objectType: 'entity' | 'value' }>()
  const readings: ParseResult['readings'] = []
  const constraints: ParseResult['constraints'] = []
  const subtypes: ParseResult['subtypes'] = []
  const deferred: Array<{ constraintText: string; readingText: string }> = []

  // Initialize nounMap with existing nouns (preserve their stored objectType)
  for (const n of existingNouns) {
    if (!nounMap.has(n.name)) {
      nounMap.set(n.name, { name: n.name, objectType: n.objectType || 'entity' })
    }
  }

  // Split on blank lines into blocks
  const blocks = text.split(/\n\s*\n/).filter(b => b.trim())

  for (const block of blocks) {
    const lines = block.split('\n')
    const factLine = lines[0].trim().replace(/\.$/, '')

    // Check for subtype declaration
    const subtypeMatch = factLine.match(/^([A-Z][a-zA-Z0-9]*)\s+is a subtype of\s+([A-Z][a-zA-Z0-9]*)/i)
    if (subtypeMatch) {
      const child = subtypeMatch[1]
      const parent = subtypeMatch[2]
      subtypes.push({ child, parent })
      // Ensure both nouns exist
      if (!nounMap.has(child)) nounMap.set(child, { name: child, objectType: 'entity' })
      if (!nounMap.has(parent)) nounMap.set(parent, { name: parent, objectType: 'entity' })
      continue
    }

    // Build current noun list for tokenization (combine existing + discovered)
    const currentNouns = [
      ...existingNouns,
      ...[...nounMap.values()]
        .filter(n => !existingNouns.some(e => e.name === n.name))
        .map(n => ({ name: n.name, id: '' })),
    ]

    // Tokenize reading
    const tokenized = tokenizeReading(factLine, currentNouns)
    let nounNames = tokenized.nounRefs.map(r => r.name)

    // PascalCase fallback if tokenization found fewer than 2 nouns
    if (nounNames.length < 2) {
      const pascalWords = factLine.match(/[A-Z][a-zA-Z0-9]*/g) || []
      nounNames = pascalWords
    }

    if (nounNames.length < 2) {
      warnings.push(`Reading "${factLine}" has fewer than 2 nouns — skipped`)
      continue
    }

    // Determine predicate
    const predicate = tokenized.predicate || extractPredicate(factLine, nounNames)
    const isHasPredicate = /^has$/i.test(predicate.trim())

    // Accumulate nouns
    for (let i = 0; i < nounNames.length; i++) {
      const name = nounNames[i]
      if (!nounMap.has(name)) {
        const objectType = (isHasPredicate && i === nounNames.length - 1) ? 'value' : 'entity'
        nounMap.set(name, { name, objectType })
      }
    }

    // Accumulate reading
    const readingText = factLine
    readings.push({ text: readingText, nouns: nounNames, predicate })

    // Parse indented constraint lines
    const constraintLines = lines.slice(1)
      .filter(l => l.match(/^\s+\S/))
      .map(l => l.trim())

    for (const constraintText of constraintLines) {
      const parsed = parseConstraintText(constraintText)
      if (!parsed) {
        warnings.push(`Unrecognized constraint pattern: "${constraintText}"`)
        continue
      }

      for (const pc of parsed) {
        // Map constraint nouns to role indices in the reading
        const roles = pc.nouns
          .map(cn => nounNames.indexOf(cn))
          .filter(idx => idx !== -1)

        if (roles.length === 0 && pc.nouns.length > 0) {
          // Constraint nouns not in this reading — defer
          deferred.push({ constraintText, readingText })
          continue
        }

        constraints.push({
          kind: pc.kind as 'UC' | 'MC' | 'RC',
          modality: pc.modality,
          reading: readingText,
          roles,
        })
      }
    }
  }

  // Retry deferred constraints against full noun/reading set
  for (const d of deferred) {
    const parsed = parseConstraintText(d.constraintText)
    if (!parsed) continue

    // Find a reading whose nouns match the constraint's nouns
    let resolved = false
    for (const reading of readings) {
      for (const pc of parsed) {
        const roles = pc.nouns
          .map(cn => reading.nouns.indexOf(cn))
          .filter(idx => idx !== -1)

        if (roles.length > 0) {
          constraints.push({
            kind: pc.kind as 'UC' | 'MC' | 'RC',
            modality: pc.modality,
            reading: reading.text,
            roles,
          })
          resolved = true
        }
      }
      if (resolved) break
    }

    if (!resolved) {
      warnings.push(`Deferred constraint still unresolved: "${d.constraintText}"`)
    }
  }

  // Build nouns array from map
  const nouns = [...nounMap.values()]

  return {
    nouns,
    readings,
    constraints,
    subtypes,
    transitions: [],
    facts: [],
    warnings,
  }
}

/** Extract predicate between first two nouns when tokenizer didn't find it. */
function extractPredicate(text: string, nounNames: string[]): string {
  if (nounNames.length < 2) return ''
  const first = text.indexOf(nounNames[0])
  if (first === -1) return ''
  const afterFirst = first + nounNames[0].length
  const second = text.indexOf(nounNames[1], afterFirst)
  if (second === -1) return ''
  return text.slice(afterFirst, second).trim()
}

// ── HTTP Handler ───────────────────────────────────────────────────

function getDB(env: Env): DurableObjectStub {
  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  return env.GRAPHDL_DB.get(id)
}

export async function handleParse(request: Request, env: Env): Promise<Response> {
  if (request.method !== 'POST') {
    return error(405, { errors: [{ message: 'Method not allowed' }] })
  }

  const body = await request.json() as { text?: string; domain?: string }
  if (!body.text) {
    return error(400, { errors: [{ message: 'text is required' }] })
  }
  if (!body.domain) {
    return error(400, { errors: [{ message: 'domain is required' }] })
  }

  // Load existing nouns for tokenization context (read-only)
  const db = getDB(env) as any
  const existingNouns = await db.findInCollection('nouns', {
    domain_id: { equals: body.domain },
  }, { limit: 10000 })
  const nouns = existingNouns.docs.map((n: any) => ({ name: n.name, id: n.id, objectType: n.objectType }))

  const result = parseFORML2(body.text, nouns)
  return json(result)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/parse.test.ts`
Expected: PASS

- [ ] **Step 5: Add tests for multi-block parsing, subtypes, and edge cases**

Append to `src/api/parse.test.ts`:

```typescript
  it('parses multiple readings separated by blank lines', () => {
    const text = `Customer has Name.
  Each Customer has at most one Name.

Customer submits SupportRequest.
  Each SupportRequest is submitted by at most one Customer.`

    const result = parseFORML2(text, [])

    expect(result.nouns).toHaveLength(3)
    expect(result.readings).toHaveLength(2)
    expect(result.constraints).toHaveLength(2)

    // Second reading reuses Customer noun
    expect(result.readings[1]).toMatchObject({
      text: 'Customer submits SupportRequest',
      nouns: ['Customer', 'SupportRequest'],
      predicate: 'submits',
    })
  })

  it('detects subtype declarations', () => {
    const text = `PremiumCustomer is a subtype of Customer.`

    const result = parseFORML2(text, [])

    expect(result.subtypes).toHaveLength(1)
    expect(result.subtypes[0]).toEqual({
      child: 'PremiumCustomer',
      parent: 'Customer',
    })
    // Both nouns should be in the noun list
    expect(result.nouns.find(n => n.name === 'PremiumCustomer')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'Customer')).toBeDefined()
  })

  it('produces partial results with warnings for malformed input', () => {
    const text = `Customer has Name.
  Each Customer has at most one Name.

justgarbage

SupportRequest has Priority.`

    const result = parseFORML2(text, [])

    // Good blocks parsed
    expect(result.readings).toHaveLength(2)
    // Bad block produces warning
    expect(result.warnings.length).toBeGreaterThanOrEqual(1)
    expect(result.warnings.some(w => w.includes('fewer than 2 nouns'))).toBe(true)
  })

  it('handles "exactly one" producing UC + MC constraints', () => {
    const text = `Organization has Name.
  Each Organization has exactly one Name.`

    const result = parseFORML2(text, [])

    expect(result.constraints).toHaveLength(2)
    expect(result.constraints.find(c => c.kind === 'UC')).toBeDefined()
    expect(result.constraints.find(c => c.kind === 'MC')).toBeDefined()
  })

  it('uses existing nouns for tokenization context', () => {
    const existingNouns = [
      { name: 'Customer', id: 'n1' },
      { name: 'Name', id: 'n2' },
    ]
    const text = `Customer has Name.
  Each Customer has at most one Name.`

    const result = parseFORML2(text, existingNouns)

    expect(result.nouns).toHaveLength(2) // No duplicates
    expect(result.readings).toHaveLength(1)
  })

  it('returns empty arrays for transitions and facts', () => {
    const text = `Customer has Name.`

    const result = parseFORML2(text, [])

    expect(result.transitions).toEqual([])
    expect(result.facts).toEqual([])
  })

  it('warns on unrecognized constraint patterns', () => {
    const text = `Customer has Name.
  This is not a valid constraint.`

    const result = parseFORML2(text, [])

    expect(result.readings).toHaveLength(1)
    expect(result.warnings).toHaveLength(1)
    expect(result.warnings[0]).toContain('Unrecognized constraint pattern')
  })

  it('handles non-"has" predicates as entity types', () => {
    const text = `Customer submits SupportRequest.`

    const result = parseFORML2(text, [])

    expect(result.nouns.find(n => n.name === 'SupportRequest')).toMatchObject({
      objectType: 'entity', // not "has" → entity, not value
    })
  })

  it('retries deferred constraints against later-defined nouns', () => {
    // Constraint on first block references nouns from second block
    const text = `Customer has Name.
  Each SupportRequest is submitted by at most one Customer.

Customer submits SupportRequest.`

    const result = parseFORML2(text, [])

    // The deferred constraint should resolve against the second reading
    expect(result.constraints.length).toBeGreaterThanOrEqual(1)
    const deferredConstraint = result.constraints.find(
      c => c.reading === 'Customer submits SupportRequest'
    )
    expect(deferredConstraint).toBeDefined()
    expect(result.warnings.filter(w => w.includes('unresolved'))).toHaveLength(0)
  })

  it('warns on permanently unresolvable deferred constraints', () => {
    const text = `Customer has Name.
  Each Order has at most one Invoice.`

    const result = parseFORML2(text, [])

    // Order and Invoice never appear as a reading → warning
    expect(result.warnings.some(w => w.includes('unresolved'))).toBe(true)
  })
```

- [ ] **Step 6: Run all parse tests**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/parse.test.ts`
Expected: All PASS

- [ ] **Step 7: Commit**

```bash
git add src/api/parse.ts src/api/parse.test.ts
git commit -m "feat: add /parse endpoint — pure-function FORML2 parser"
```

---

### Task 2: Register `/parse` route

**Files:**
- Modify: `src/api/router.ts:219-223`

- [ ] **Step 1: Add the import and route registration**

In `src/api/router.ts`, add the import at the top and register the route before the `*` fallback:

```typescript
// Add import (after existing imports, around line 7):
import { handleParse } from './parse'

// Add route (before the 404 fallback at line 223):
router.all('/parse', handleParse)
```

The route goes at root level (like `/seed` and `/claims`), not under `/api/`.

- [ ] **Step 2: Run the parse tests to verify nothing broke**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/parse.test.ts`
Expected: All PASS

- [ ] **Step 3: Commit**

```bash
git add src/api/router.ts
git commit -m "feat: register /parse route in router"
```

---

## Chunk 2: `/verify` endpoint

### Task 3: Verify handler — deterministic noun matching

**Files:**
- Create: `src/api/verify.ts`
- Test: `src/api/verify.test.ts`

The `/verify` handler loads domain data (readings, constraints, constraint-spans, roles, nouns) and tokenizes prose text to find noun-type mentions. It classifies constraints as "in scope" (their nouns appear in prose) or "unmatched" (their nouns don't appear).

It does NOT attempt to extract structured facts or detect violations — that requires LLM analysis (apis side). It identifies which constraints are *relevant* to the text.

#### Algorithm

1. Load all nouns, readings, roles, constraints, constraint-spans for the domain.
2. Tokenize the prose once against all domain nouns.
3. A reading "matches" if at least one of its nouns appears in the prose. The `nouns` array in the match reports which nouns were found (may be a subset — the downstream LLM does deeper analysis).
4. Build a mapping: constraint → its reading (via constraint-spans → roles → reading).
5. Classify each constraint:
   - If its reading matched → deterministically checkable (in scope).
   - If its reading didn't match → unmatched (needs semantic analysis).
6. Return `{ matches, unmatchedConstraints }`.

- [ ] **Step 1: Write the failing test for basic verification**

Create `src/api/verify.test.ts`:

```typescript
import { describe, it, expect } from 'vitest'
import { verifyProse } from './verify'

describe('verifyProse', () => {
  it('matches readings whose nouns appear in prose', () => {
    const domainData = {
      nouns: [
        { id: 'n1', name: 'Customer' },
        { id: 'n2', name: 'Name' },
        { id: 'n3', name: 'SupportRequest' },
        { id: 'n4', name: 'Priority' },
      ],
      readings: [
        { id: 'r1', text: 'Customer has Name' },
        { id: 'r2', text: 'SupportRequest has Priority' },
      ],
      roles: [
        { id: 'ro1', reading: 'r1', noun: 'n1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', roleIndex: 1 },
        { id: 'ro3', reading: 'r2', noun: 'n3', roleIndex: 0 },
        { id: 'ro4', reading: 'r2', noun: 'n4', roleIndex: 1 },
      ],
      constraints: [
        { id: 'c1', kind: 'UC', text: 'Each Customer has at most one Name' },
        { id: 'c2', kind: 'UC', text: 'Each SupportRequest has at most one Priority' },
      ],
      constraintSpans: [
        { constraint: 'c1', role: 'ro1' },
        { constraint: 'c2', role: 'ro3' },
      ],
    }

    const prose = 'The Customer named John submitted a request.'

    const result = verifyProse(prose, domainData)

    // Customer and Name appear → reading r1 matches
    expect(result.matches).toHaveLength(1)
    expect(result.matches[0]).toMatchObject({
      reading: 'Customer has Name',
      nouns: ['Customer'],
    })

    // SupportRequest and Priority don't both appear → c2 unmatched
    expect(result.unmatchedConstraints).toContain(
      'Each SupportRequest has at most one Priority'
    )
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/verify.test.ts`
Expected: FAIL — `verifyProse` does not exist

- [ ] **Step 3: Write the `verifyProse` pure function and HTTP handler**

Create `src/api/verify.ts`:

```typescript
import { json, error } from 'itty-router'
import type { Env } from '../types'
import { tokenizeReading } from '../claims/tokenize'

interface DomainData {
  nouns: Array<{ id: string; name: string }>
  readings: Array<{ id: string; text: string }>
  roles: Array<{ id: string; reading: string; noun: string; roleIndex: number }>
  constraints: Array<{ id: string; kind: string; text?: string }>
  constraintSpans: Array<{ constraint: string; role: string }>
}

interface VerifyResult {
  matches: Array<{
    reading: string
    nouns: string[]
  }>
  unmatchedConstraints: string[]
}

/**
 * Pure-function prose verifier.
 *
 * Tokenizes prose against domain nouns to find which readings' noun types
 * are mentioned. Classifies constraints as deterministically checkable
 * (nouns present) or unmatched (nouns absent — needs semantic analysis).
 */
export function verifyProse(prose: string, data: DomainData): VerifyResult {
  const matches: VerifyResult['matches'] = []
  const matchedReadingIds = new Set<string>()

  // Tokenize prose once against all domain nouns (result is the same every iteration)
  const tokenized = tokenizeReading(prose, data.nouns)
  const foundNounNames = new Set(tokenized.nounRefs.map(r => r.name))

  // For each reading, check if its nouns appear in the prose
  for (const reading of data.readings) {
    // Get nouns for this reading via roles
    const readingRoles = data.roles
      .filter(r => r.reading === reading.id)
      .sort((a, b) => a.roleIndex - b.roleIndex)

    const readingNouns = readingRoles
      .map(r => data.nouns.find(n => n.id === r.noun))
      .filter((n): n is { id: string; name: string } => !!n)

    // Check which of this reading's nouns appear in the prose
    const matchedNouns = readingNouns.filter(n => foundNounNames.has(n.name))

    if (matchedNouns.length > 0) {
      matches.push({
        reading: reading.text,
        nouns: matchedNouns.map(n => n.name),
      })
      matchedReadingIds.add(reading.id)
    }
  }

  // Build constraint → reading mapping via spans → roles
  const unmatchedConstraints: string[] = []

  for (const constraint of data.constraints) {
    // Find which roles this constraint spans
    const spans = data.constraintSpans.filter(s => s.constraint === constraint.id)
    const roleIds = spans.map(s => s.role)

    // Find which reading(s) those roles belong to
    const readingIds = new Set(
      roleIds
        .map(rid => data.roles.find(r => r.id === rid)?.reading)
        .filter((rid): rid is string => !!rid)
    )

    // If none of the constraint's readings matched, it's unmatched
    const anyMatched = [...readingIds].some(rid => matchedReadingIds.has(rid))
    if (!anyMatched) {
      unmatchedConstraints.push(constraint.text || `${constraint.kind} constraint ${constraint.id}`)
    }
  }

  return { matches, unmatchedConstraints }
}

// ── HTTP Handler ───────────────────────────────────────────────────

function getDB(env: Env): DurableObjectStub {
  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  return env.GRAPHDL_DB.get(id)
}

export async function handleVerify(request: Request, env: Env): Promise<Response> {
  if (request.method !== 'POST') {
    return error(405, { errors: [{ message: 'Method not allowed' }] })
  }

  const body = await request.json() as { text?: string; domain?: string }
  if (!body.text) {
    return error(400, { errors: [{ message: 'text is required' }] })
  }
  if (!body.domain) {
    return error(400, { errors: [{ message: 'domain is required' }] })
  }

  const db = getDB(env) as any
  const domainId = body.domain

  // Load all domain data in parallel (read-only)
  // Note: roles and constraint-spans don't have a domain_id column — they're
  // child records of domain-scoped readings/constraints. We load all and filter
  // in-memory by matching against the domain-scoped reading IDs.
  const [nounsResult, readingsResult, rolesResult, constraintsResult, spansResult] = await Promise.all([
    db.findInCollection('nouns', { domain_id: { equals: domainId } }, { limit: 10000 }),
    db.findInCollection('readings', { domain_id: { equals: domainId } }, { limit: 10000 }),
    db.findInCollection('roles', {}, { limit: 10000 }),
    db.findInCollection('constraints', { domain_id: { equals: domainId } }, { limit: 10000 }),
    db.findInCollection('constraint-spans', {}, { limit: 10000 }),
  ])

  const domainData: DomainData = {
    nouns: nounsResult.docs.map((n: any) => ({ id: n.id, name: n.name })),
    readings: readingsResult.docs.map((r: any) => ({ id: r.id, text: r.text })),
    roles: rolesResult.docs
      .filter((r: any) => readingsResult.docs.some((rd: any) => rd.id === r.reading))
      .map((r: any) => ({ id: r.id, reading: r.reading, noun: r.noun, roleIndex: r.roleIndex })),
    constraints: constraintsResult.docs.map((c: any) => ({ id: c.id, kind: c.kind, text: c.text })),
    constraintSpans: spansResult.docs.map((s: any) => ({ constraint: s.constraint, role: s.role })),
  }

  const result = verifyProse(body.text, domainData)
  return json(result)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/verify.test.ts`
Expected: PASS

- [ ] **Step 5: Add tests for all-matched, all-unmatched, and mixed scenarios**

Append to `src/api/verify.test.ts`:

```typescript
  it('returns all constraints as unmatched when prose has no domain nouns', () => {
    const domainData = {
      nouns: [
        { id: 'n1', name: 'Customer' },
        { id: 'n2', name: 'Name' },
      ],
      readings: [
        { id: 'r1', text: 'Customer has Name' },
      ],
      roles: [
        { id: 'ro1', reading: 'r1', noun: 'n1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', roleIndex: 1 },
      ],
      constraints: [
        { id: 'c1', kind: 'UC', text: 'Each Customer has at most one Name' },
      ],
      constraintSpans: [
        { constraint: 'c1', role: 'ro1' },
      ],
    }

    const prose = 'The weather is nice today.'

    const result = verifyProse(prose, domainData)

    expect(result.matches).toHaveLength(0)
    expect(result.unmatchedConstraints).toHaveLength(1)
  })

  it('matches all constraints when all nouns appear in prose', () => {
    const domainData = {
      nouns: [
        { id: 'n1', name: 'Customer' },
        { id: 'n2', name: 'Name' },
      ],
      readings: [
        { id: 'r1', text: 'Customer has Name' },
      ],
      roles: [
        { id: 'ro1', reading: 'r1', noun: 'n1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', roleIndex: 1 },
      ],
      constraints: [
        { id: 'c1', kind: 'UC', text: 'Each Customer has at most one Name' },
      ],
      constraintSpans: [
        { constraint: 'c1', role: 'ro1' },
      ],
    }

    const prose = 'The Customer has a Name of "John Smith".'

    const result = verifyProse(prose, domainData)

    expect(result.matches).toHaveLength(1)
    expect(result.unmatchedConstraints).toHaveLength(0)
  })

  it('handles constraints with no text gracefully', () => {
    const domainData = {
      nouns: [
        { id: 'n1', name: 'Customer' },
        { id: 'n2', name: 'Name' },
      ],
      readings: [
        { id: 'r1', text: 'Customer has Name' },
      ],
      roles: [
        { id: 'ro1', reading: 'r1', noun: 'n1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', roleIndex: 1 },
      ],
      constraints: [
        { id: 'c1', kind: 'UC' }, // no text field
      ],
      constraintSpans: [
        { constraint: 'c1', role: 'ro1' },
      ],
    }

    const prose = 'Hello world.'

    const result = verifyProse(prose, domainData)

    expect(result.unmatchedConstraints).toHaveLength(1)
    expect(result.unmatchedConstraints[0]).toContain('UC constraint c1')
  })

  it('handles empty domain data', () => {
    const domainData = {
      nouns: [],
      readings: [],
      roles: [],
      constraints: [],
      constraintSpans: [],
    }

    const result = verifyProse('Some text', domainData)

    expect(result.matches).toEqual([])
    expect(result.unmatchedConstraints).toEqual([])
  })

  it('correctly splits matched and unmatched constraints in mixed prose', () => {
    const domainData = {
      nouns: [
        { id: 'n1', name: 'Customer' },
        { id: 'n2', name: 'Name' },
        { id: 'n3', name: 'SupportRequest' },
        { id: 'n4', name: 'Priority' },
        { id: 'n5', name: 'Invoice' },
        { id: 'n6', name: 'Amount' },
      ],
      readings: [
        { id: 'r1', text: 'Customer has Name' },
        { id: 'r2', text: 'SupportRequest has Priority' },
        { id: 'r3', text: 'Invoice has Amount' },
      ],
      roles: [
        { id: 'ro1', reading: 'r1', noun: 'n1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', roleIndex: 1 },
        { id: 'ro3', reading: 'r2', noun: 'n3', roleIndex: 0 },
        { id: 'ro4', reading: 'r2', noun: 'n4', roleIndex: 1 },
        { id: 'ro5', reading: 'r3', noun: 'n5', roleIndex: 0 },
        { id: 'ro6', reading: 'r3', noun: 'n6', roleIndex: 1 },
      ],
      constraints: [
        { id: 'c1', kind: 'UC', text: 'Each Customer has at most one Name' },
        { id: 'c2', kind: 'UC', text: 'Each SupportRequest has at most one Priority' },
        { id: 'c3', kind: 'MC', text: 'Each Invoice has at least one Amount' },
      ],
      constraintSpans: [
        { constraint: 'c1', role: 'ro1' },
        { constraint: 'c2', role: 'ro3' },
        { constraint: 'c3', role: 'ro5' },
      ],
    }

    // Prose mentions Customer and SupportRequest, but not Invoice
    const prose = 'The Customer submitted a SupportRequest about billing.'

    const result = verifyProse(prose, domainData)

    // r1 and r2 match (Customer, SupportRequest mentioned), r3 doesn't (Invoice not mentioned)
    expect(result.matches).toHaveLength(2)
    expect(result.matches.map(m => m.reading)).toContain('Customer has Name')
    expect(result.matches.map(m => m.reading)).toContain('SupportRequest has Priority')

    // c3 (Invoice constraint) is unmatched
    expect(result.unmatchedConstraints).toHaveLength(1)
    expect(result.unmatchedConstraints[0]).toContain('Invoice')
  })
```

- [ ] **Step 6: Run all verify tests**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/verify.test.ts`
Expected: All PASS

- [ ] **Step 7: Commit**

```bash
git add src/api/verify.ts src/api/verify.test.ts
git commit -m "feat: add /verify endpoint — deterministic noun matching"
```

---

### Task 4: Register `/verify` route and run full test suite

**Files:**
- Modify: `src/api/router.ts:219-223`

- [ ] **Step 1: Add the import and route registration**

In `src/api/router.ts`, add the import and route (alongside `/parse`):

```typescript
// Add import (with the parse import):
import { handleVerify } from './verify'

// Add route (before the 404 fallback, after /parse):
router.all('/verify', handleVerify)
```

Final route section should look like:

```typescript
// ── Seed / Claims ───────────────────────────────────────────────────
router.all('/seed', handleSeed)
router.all('/claims', handleSeed) // Alias used by apis worker

// ── Parse / Verify ──────────────────────────────────────────────────
router.all('/parse', handleParse)
router.all('/verify', handleVerify)

// ── 404 fallback ─────────────────────────────────────────────────────
router.all('*', () => error(404, { errors: [{ message: 'Not Found' }] }))
```

- [ ] **Step 2: Run the full test suite**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run`
Expected: All tests pass (parse, verify, integration, any existing tests)

- [ ] **Step 3: Commit**

```bash
git add src/api/router.ts
git commit -m "feat: register /verify route in router"
```

- [ ] **Step 4: Run TypeScript type check**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx tsc --noEmit`
Expected: No type errors (or only pre-existing ones)
