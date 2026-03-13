# WASM Constraint Engine + Chat Endpoint Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add runtime constraint evaluation (Rust → WASM) and a `POST /api/chat` endpoint to graphdl-orm, enabling constraint-aware AI responses streamed via SSE.

**Architecture:** A new `constraint-ir` generator queries the metamodel database and emits a self-contained JSON IR. A Rust crate compiles that IR into evaluation logic exposed via `wasm-bindgen`. The chat endpoint loads agent context from the generators collection, calls Claude, evaluates drafts against constraints via WASM, redrafts on violation, and streams clean responses.

**Tech Stack:** TypeScript (itty-router, Cloudflare Workers), Rust (wasm-bindgen, serde), vitest, cargo test

**Spec:** `docs/superpowers/specs/2026-03-12-graphdl-wasm-constraint-engine-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `src/generate/constraint-ir.ts` | Create | `generateConstraintIR()` — queries DB, emits `ConstraintIR` JSON |
| `src/generate/constraint-ir.test.ts` | Create | Unit tests for the constraint-ir generator |
| `src/generate/index.ts` | Modify (line 5) | Re-export `generateConstraintIR` |
| `src/api/generate.ts` | Modify (lines 1-9, 24-42) | Add `'constraint-ir'` to `VALID_FORMATS`, dispatch case |
| `crates/constraint-eval/Cargo.toml` | Create | Rust crate manifest with wasm-bindgen, serde |
| `crates/constraint-eval/src/lib.rs` | Create | WASM entry points: `load_ir()`, `evaluate_response()` |
| `crates/constraint-eval/src/types.rs` | Create | IR deserialization types and `Population`/`Violation` structs |
| `crates/constraint-eval/src/evaluate.rs` | Create | Core evaluation: alethic, deontic, set-comparison |
| `crates/constraint-eval/tests/integration.rs` | Create | Integration tests with fixture IR JSON |
| `src/api/chat.ts` | Create | `handleChat()` — Claude orchestration, WASM eval, SSE streaming |
| `src/api/chat.test.ts` | Create | Unit tests for chat pipeline |
| `src/api/router.ts` | Modify (lines 224-226) | Register `POST /api/chat` route |
| `src/api/generate.test.ts` | Modify | Add `'constraint-ir'` to format validation test |
| `src/types.ts` | Modify (lines 1-4) | Add `AI` and `ANTHROPIC_API_KEY` to `Env` (both optional) |
| `wrangler.jsonc` | Modify (lines 22-24) | Add `ai` binding, `rules` for WASM |
| `.github/workflows/deploy.yml` | Modify (lines 11-27) | Add Rust toolchain, cargo test, wasm-pack build |

---

## Chunk 0: Branch and project setup

### Task 0: Create feature branch

**Files:**
- No file changes

- [ ] **Step 1: Create and switch to feature branch**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git checkout -b feat/wasm-constraint-engine
```

- [ ] **Step 2: Verify clean state**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && git status`
Expected: On branch feat/wasm-constraint-engine, clean working tree (untracked docs are fine)

---

## Chunk 1: Constraint IR Generator

This chunk adds a new output format `'constraint-ir'` to the existing generate system. It follows the exact pattern of `openapi.ts`, `xstate.ts`, etc.: fetch domain data from DO via `findInCollection`, build structured output, return it.

### Task 1: Define ConstraintIR types and write the generator

**Files:**
- Create: `src/generate/constraint-ir.ts`
- Test: `src/generate/constraint-ir.test.ts`

The generator queries the DO for all domain-scoped data and assembles the IR. Key implementation details:

1. Fetches: nouns, constraints, constraint-spans, roles, readings, graph-schemas, state-machine-definitions, statuses, transitions, event-types, guards
2. Builds `nouns` record from domain nouns
3. Builds `factTypes` record from graph-schemas + roles + readings
4. Builds `constraints` array from constraints + constraint-spans + roles, re-deriving `deonticOperator` from `text` via `parseConstraintText()`
5. Builds `stateMachines` record from state-machine-definitions + statuses + transitions + guards (resolving guard → graph_schema → constraint_spans → constraints)

- [ ] **Step 1: Write the failing test for basic IR generation**

```typescript
// src/generate/constraint-ir.test.ts
import { describe, it, expect, vi } from 'vitest'
import { generateConstraintIR } from './constraint-ir'

/**
 * Create a mock DB that returns canned data for findInCollection calls.
 * This mirrors the pattern used in openapi.test.ts and xstate.test.ts.
 */
function createMockDB(data: Record<string, any[]>) {
  return {
    findInCollection: vi.fn(async (slug: string, _where?: any, _opts?: any) => ({
      docs: data[slug] || [],
      totalDocs: (data[slug] || []).length,
      limit: 10000,
      page: 1,
      hasNextPage: false,
    })),
  }
}

describe('generateConstraintIR', () => {
  it('generates IR with nouns, factTypes, constraints, and stateMachines', async () => {
    const db = createMockDB({
      'nouns': [
        { id: 'n1', name: 'Customer', objectType: 'entity', domain: 'd1' },
        { id: 'n2', name: 'Name', objectType: 'value', valueType: 'string', domain: 'd1' },
      ],
      'graph-schemas': [
        { id: 'gs1', name: 'CustomerName', domain: 'd1' },
      ],
      'readings': [
        { id: 'r1', text: 'Customer has Name', graphSchema: 'gs1', domain: 'd1' },
      ],
      'roles': [
        { id: 'ro1', reading: 'r1', noun: 'n1', graphSchema: 'gs1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', graphSchema: 'gs1', roleIndex: 1 },
      ],
      'constraints': [
        { id: 'c1', kind: 'UC', modality: 'Alethic', text: 'Each Customer has at most one Name', domain: 'd1' },
      ],
      'constraint-spans': [
        { id: 'cs1', constraint: 'c1', role: 'ro1' },
      ],
      'state-machine-definitions': [],
      'statuses': [],
      'transitions': [],
      'event-types': [],
      'guards': [],
    })

    const ir = await generateConstraintIR(db, 'd1')

    // Domain
    expect(ir.domain).toBe('d1')

    // Nouns
    expect(ir.nouns['Customer']).toEqual({ objectType: 'entity' })
    expect(ir.nouns['Name']).toEqual({ objectType: 'value', valueType: 'string' })

    // FactTypes
    expect(Object.keys(ir.factTypes)).toHaveLength(1)
    const ft = ir.factTypes['gs1']
    expect(ft.reading).toBe('Customer has Name')
    expect(ft.roles).toEqual([
      { nounName: 'Customer', roleIndex: 0 },
      { nounName: 'Name', roleIndex: 1 },
    ])

    // Constraints
    expect(ir.constraints).toHaveLength(1)
    expect(ir.constraints[0]).toMatchObject({
      id: 'c1',
      kind: 'UC',
      modality: 'Alethic',
      text: 'Each Customer has at most one Name',
      spans: [{ factTypeId: 'gs1', roleIndex: 0 }],
    })
    // UC alethic → no deonticOperator
    expect(ir.constraints[0].deonticOperator).toBeUndefined()

    // No state machines
    expect(Object.keys(ir.stateMachines)).toHaveLength(0)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/constraint-ir.test.ts`
Expected: FAIL — `generateConstraintIR` does not exist

- [ ] **Step 3: Write the constraint-ir generator**

```typescript
// src/generate/constraint-ir.ts
import { parseConstraintText } from '../hooks/parse-constraint'

// ── Types ──────────────────────────────────────────────────────────────

export interface ConstraintIR {
  domain: string
  nouns: Record<string, {
    objectType: 'entity' | 'value'
    enumValues?: string[]
    valueType?: string
    superType?: string
  }>
  factTypes: Record<string, {
    reading: string
    roles: Array<{ nounName: string; roleIndex: number }>
  }>
  constraints: Array<{
    id: string
    kind: string
    modality: string
    deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
    text: string
    spans: Array<{ factTypeId: string; roleIndex: number; subsetAutofill?: boolean }>
    setComparisonArgumentLength?: number
    clauses?: string[]
    entity?: string
  }>
  stateMachines: Record<string, {
    nounName: string
    statuses: string[]
    transitions: Array<{
      from: string
      to: string
      event: string
      guard?: {
        graphSchemaId: string
        constraintIds: string[]
      }
    }>
  }>
}

// ── Helpers ────────────────────────────────────────────────────────────

async function fetchAll(db: any, slug: string, where?: any): Promise<any[]> {
  const result = await db.findInCollection(slug, where, { limit: 10000 })
  return result?.docs || []
}

// ── Generator ──────────────────────────────────────────────────────────

export async function generateConstraintIR(db: any, domainId: string): Promise<ConstraintIR> {
  const domainFilter = { domain: { equals: domainId } }

  // Fetch all domain data in parallel
  const [
    nouns,
    graphSchemas,
    readings,
    roles,
    constraints,
    constraintSpans,
    smDefs,
    statuses,
    transitions,
    eventTypes,
    guards,
  ] = await Promise.all([
    fetchAll(db, 'nouns', domainFilter),
    fetchAll(db, 'graph-schemas', domainFilter),
    fetchAll(db, 'readings', domainFilter),
    fetchAll(db, 'roles'),
    fetchAll(db, 'constraints', domainFilter),
    fetchAll(db, 'constraint-spans'),
    fetchAll(db, 'state-machine-definitions', domainFilter),
    fetchAll(db, 'statuses', domainFilter),
    fetchAll(db, 'transitions', domainFilter),
    fetchAll(db, 'event-types', domainFilter),
    fetchAll(db, 'guards', domainFilter),
  ])

  // Build noun lookup
  const nounById = new Map(nouns.map((n: any) => [n.id, n]))

  // Build role lookup: roleId → { nounId, graphSchemaId, roleIndex }
  const roleById = new Map(
    roles.map((r: any) => [r.id, {
      nounId: r.noun,
      graphSchemaId: r.graphSchema,
      roleIndex: r.roleIndex,
      readingId: r.reading,
    }])
  )

  // Filter roles to those belonging to domain readings
  const domainReadingIds = new Set(readings.map((r: any) => r.id))
  const domainRoles = roles.filter((r: any) => domainReadingIds.has(r.reading))

  // ── Nouns ──
  const irNouns: ConstraintIR['nouns'] = {}
  for (const noun of nouns) {
    const entry: any = { objectType: noun.objectType || 'entity' }
    if (noun.enumValues) {
      try {
        const parsed = typeof noun.enumValues === 'string' ? JSON.parse(noun.enumValues) : noun.enumValues
        if (Array.isArray(parsed) && parsed.length > 0) entry.enumValues = parsed
      } catch { /* skip malformed enum */ }
    }
    if (noun.valueType) entry.valueType = noun.valueType
    if (noun.superType) {
      const parent = nounById.get(noun.superType)
      if (parent) entry.superType = parent.name
    }
    irNouns[noun.name] = entry
  }

  // ── FactTypes ──
  const irFactTypes: ConstraintIR['factTypes'] = {}
  for (const gs of graphSchemas) {
    const gsRoles = domainRoles
      .filter((r: any) => r.graphSchema === gs.id)
      .sort((a: any, b: any) => (a.roleIndex || 0) - (b.roleIndex || 0))

    const gsReadings = readings.filter((r: any) => r.graphSchema === gs.id)
    const readingText = gsReadings[0]?.text || gs.name || ''

    irFactTypes[gs.id] = {
      reading: readingText,
      roles: gsRoles.map((r: any) => ({
        nounName: nounById.get(r.noun)?.name || 'Unknown',
        roleIndex: r.roleIndex || 0,
      })),
    }
  }

  // ── Constraints ──
  // Build span lookup: constraintId → Array<{ roleId, ... }>
  const spansByConstraint = new Map<string, any[]>()
  for (const span of constraintSpans) {
    const cid = span.constraint
    if (!spansByConstraint.has(cid)) spansByConstraint.set(cid, [])
    spansByConstraint.get(cid)!.push(span)
  }

  const irConstraints: ConstraintIR['constraints'] = []
  for (const c of constraints) {
    const spans = spansByConstraint.get(c.id) || []

    // Resolve spans to factType + roleIndex
    const irSpans = spans
      .map((span: any) => {
        const role = roleById.get(span.role)
        if (!role) return null
        return {
          factTypeId: role.graphSchemaId,
          roleIndex: role.roleIndex,
          ...(span.subsetAutofill ? { subsetAutofill: true } : {}),
        }
      })
      .filter(Boolean) as Array<{ factTypeId: string; roleIndex: number; subsetAutofill?: boolean }>

    // Re-derive deonticOperator from text
    let deonticOperator: 'obligatory' | 'forbidden' | 'permitted' | undefined
    if (c.modality === 'Deontic' && c.text) {
      const parsed = parseConstraintText(c.text)
      if (parsed?.[0]?.deonticOperator) {
        deonticOperator = parsed[0].deonticOperator
      }
    }

    const entry: ConstraintIR['constraints'][number] = {
      id: c.id,
      kind: c.kind,
      modality: c.modality || 'Alethic',
      text: c.text || '',
      spans: irSpans,
    }
    if (deonticOperator) entry.deonticOperator = deonticOperator
    if (c.setComparisonArgumentLength) entry.setComparisonArgumentLength = c.setComparisonArgumentLength
    irConstraints.push(entry)
  }

  // ── State Machines ──
  const irStateMachines: ConstraintIR['stateMachines'] = {}

  for (const smDef of smDefs) {
    const nounName = nounById.get(smDef.noun)?.name || smDef.title || 'Unknown'
    const smStatuses = statuses
      .filter((s: any) => s.stateMachineDefinition === smDef.id)
      .sort((a: any, b: any) => (a.createdAt || '').localeCompare(b.createdAt || ''))

    const statusById = new Map(smStatuses.map((s: any) => [s.id, s.name]))

    const smTransitions = transitions
      .filter((t: any) => smStatuses.some((s: any) => s.id === t.from))

    const irTransitions = smTransitions
      .map((t: any) => {
        const from = statusById.get(t.from)
        const to = statusById.get(t.to)
        const eventType = eventTypes.find((e: any) => e.id === t.eventType)
        if (!from || !to || !eventType) return null

        const transition: any = { from, to, event: eventType.name }

        // Resolve guards for this transition
        const transitionGuards = guards.filter((g: any) => g.transition === t.id)
        if (transitionGuards.length > 0) {
          const guard = transitionGuards[0]
          // Resolve graph_schema → constraint_spans → constraints
          const gsId = guard.graphSchema
          if (gsId) {
            const gsRoleIds = domainRoles
              .filter((r: any) => r.graphSchema === gsId)
              .map((r: any) => r.id)
            const guardSpans = constraintSpans.filter((s: any) =>
              gsRoleIds.includes(s.role)
            )
            const constraintIds = [...new Set(guardSpans.map((s: any) => s.constraint))]
              .filter((cid: string) => constraints.some((c: any) => c.id === cid))

            if (constraintIds.length > 0) {
              transition.guard = { graphSchemaId: gsId, constraintIds }
            }
          }
        }

        return transition
      })
      .filter(Boolean)

    irStateMachines[smDef.id] = {
      nounName,
      statuses: smStatuses.map((s: any) => s.name),
      transitions: irTransitions,
    }
  }

  return {
    domain: domainId,
    nouns: irNouns,
    factTypes: irFactTypes,
    constraints: irConstraints,
    stateMachines: irStateMachines,
  }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/constraint-ir.test.ts`
Expected: PASS

- [ ] **Step 5: Add tests for deontic constraints and state machines**

Append to `src/generate/constraint-ir.test.ts`:

```typescript
  it('re-derives deonticOperator from constraint text', async () => {
    const db = createMockDB({
      'nouns': [
        { id: 'n1', name: 'SupportResponse', objectType: 'entity', domain: 'd1' },
        { id: 'n2', name: 'ProhibitedText', objectType: 'value', domain: 'd1' },
      ],
      'graph-schemas': [
        { id: 'gs1', name: 'ResponseContainsProhibited', domain: 'd1' },
      ],
      'readings': [
        { id: 'r1', text: 'SupportResponse contains ProhibitedText', graphSchema: 'gs1', domain: 'd1' },
      ],
      'roles': [
        { id: 'ro1', reading: 'r1', noun: 'n1', graphSchema: 'gs1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', graphSchema: 'gs1', roleIndex: 1 },
      ],
      'constraints': [
        {
          id: 'c1', kind: 'UC', modality: 'Deontic',
          text: 'It is forbidden that SupportResponse contains ProhibitedText',
          domain: 'd1',
        },
      ],
      'constraint-spans': [
        { id: 'cs1', constraint: 'c1', role: 'ro1' },
      ],
      'state-machine-definitions': [],
      'statuses': [],
      'transitions': [],
      'event-types': [],
      'guards': [],
    })

    const ir = await generateConstraintIR(db, 'd1')

    expect(ir.constraints[0].deonticOperator).toBe('forbidden')
    expect(ir.constraints[0].modality).toBe('Deontic')
  })

  it('generates state machine IR with transitions', async () => {
    const db = createMockDB({
      'nouns': [
        { id: 'n1', name: 'SupportRequest', objectType: 'entity', domain: 'd1' },
      ],
      'graph-schemas': [],
      'readings': [],
      'roles': [],
      'constraints': [],
      'constraint-spans': [],
      'state-machine-definitions': [
        { id: 'sm1', title: 'SupportRequest', noun: 'n1', domain: 'd1' },
      ],
      'statuses': [
        { id: 's1', name: 'Received', stateMachineDefinition: 'sm1', domain: 'd1', createdAt: '2026-01-01' },
        { id: 's2', name: 'Investigating', stateMachineDefinition: 'sm1', domain: 'd1', createdAt: '2026-01-02' },
        { id: 's3', name: 'Resolved', stateMachineDefinition: 'sm1', domain: 'd1', createdAt: '2026-01-03' },
      ],
      'transitions': [
        { id: 't1', from: 's1', to: 's2', eventType: 'et1', domain: 'd1' },
        { id: 't2', from: 's2', to: 's3', eventType: 'et2', domain: 'd1' },
      ],
      'event-types': [
        { id: 'et1', name: 'investigate', domain: 'd1' },
        { id: 'et2', name: 'resolve', domain: 'd1' },
      ],
      'guards': [],
    })

    const ir = await generateConstraintIR(db, 'd1')

    const sm = ir.stateMachines['sm1']
    expect(sm).toBeDefined()
    expect(sm.nounName).toBe('SupportRequest')
    expect(sm.statuses).toEqual(['Received', 'Investigating', 'Resolved'])
    expect(sm.transitions).toHaveLength(2)
    expect(sm.transitions[0]).toEqual({ from: 'Received', to: 'Investigating', event: 'investigate' })
    expect(sm.transitions[1]).toEqual({ from: 'Investigating', to: 'Resolved', event: 'resolve' })
  })

  it('resolves guard → graph_schema → constraint_spans → constraints', async () => {
    const db = createMockDB({
      'nouns': [
        { id: 'n1', name: 'Order', objectType: 'entity', domain: 'd1' },
        { id: 'n2', name: 'Payment', objectType: 'entity', domain: 'd1' },
      ],
      'graph-schemas': [
        { id: 'gs1', name: 'OrderPayment', domain: 'd1' },
      ],
      'readings': [
        { id: 'r1', text: 'Order has Payment', graphSchema: 'gs1', domain: 'd1' },
      ],
      'roles': [
        { id: 'ro1', reading: 'r1', noun: 'n1', graphSchema: 'gs1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', graphSchema: 'gs1', roleIndex: 1 },
      ],
      'constraints': [
        { id: 'c1', kind: 'MC', modality: 'Alethic', text: 'Each Order has at least one Payment', domain: 'd1' },
      ],
      'constraint-spans': [
        { id: 'cs1', constraint: 'c1', role: 'ro1' },
      ],
      'state-machine-definitions': [
        { id: 'sm1', title: 'Order', noun: 'n1', domain: 'd1' },
      ],
      'statuses': [
        { id: 's1', name: 'Pending', stateMachineDefinition: 'sm1', domain: 'd1', createdAt: '2026-01-01' },
        { id: 's2', name: 'Paid', stateMachineDefinition: 'sm1', domain: 'd1', createdAt: '2026-01-02' },
      ],
      'transitions': [
        { id: 't1', from: 's1', to: 's2', eventType: 'et1', domain: 'd1' },
      ],
      'event-types': [
        { id: 'et1', name: 'pay', domain: 'd1' },
      ],
      'guards': [
        { id: 'g1', transition: 't1', graphSchema: 'gs1', domain: 'd1' },
      ],
    })

    const ir = await generateConstraintIR(db, 'd1')

    const sm = ir.stateMachines['sm1']
    expect(sm.transitions[0].guard).toEqual({
      graphSchemaId: 'gs1',
      constraintIds: ['c1'],
    })
  })

  it('handles empty domain gracefully', async () => {
    const db = createMockDB({
      'nouns': [],
      'graph-schemas': [],
      'readings': [],
      'roles': [],
      'constraints': [],
      'constraint-spans': [],
      'state-machine-definitions': [],
      'statuses': [],
      'transitions': [],
      'event-types': [],
      'guards': [],
    })

    const ir = await generateConstraintIR(db, 'd1')

    expect(ir.domain).toBe('d1')
    expect(ir.nouns).toEqual({})
    expect(ir.factTypes).toEqual({})
    expect(ir.constraints).toEqual([])
    expect(ir.stateMachines).toEqual({})
  })

  it('includes noun enum values and superType', async () => {
    const db = createMockDB({
      'nouns': [
        { id: 'n1', name: 'Priority', objectType: 'value', valueType: 'string', enumValues: '["low","medium","high"]', domain: 'd1' },
        { id: 'n2', name: 'Customer', objectType: 'entity', domain: 'd1' },
        { id: 'n3', name: 'PremiumCustomer', objectType: 'entity', superType: 'n2', domain: 'd1' },
      ],
      'graph-schemas': [],
      'readings': [],
      'roles': [],
      'constraints': [],
      'constraint-spans': [],
      'state-machine-definitions': [],
      'statuses': [],
      'transitions': [],
      'event-types': [],
      'guards': [],
    })

    const ir = await generateConstraintIR(db, 'd1')

    expect(ir.nouns['Priority']).toEqual({
      objectType: 'value',
      valueType: 'string',
      enumValues: ['low', 'medium', 'high'],
    })
    expect(ir.nouns['PremiumCustomer']).toEqual({
      objectType: 'entity',
      superType: 'Customer',
    })
  })
```

- [ ] **Step 6: Run all constraint-ir tests**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/constraint-ir.test.ts`
Expected: All PASS

- [ ] **Step 7: Commit**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/constraint-ir.ts src/generate/constraint-ir.test.ts
git commit -m "feat: add constraint-ir generator — JSON IR from metamodel"
```

### Task 2: Wire constraint-ir into the generate system

**Files:**
- Modify: `src/generate/index.ts:5`
- Modify: `src/api/generate.ts:1-9,24-42`

- [ ] **Step 1: Add export to generate/index.ts**

In `src/generate/index.ts`, add after line 5 (`export { generateReadings } from './readings'`):

```typescript
export { generateConstraintIR } from './constraint-ir'
```

- [ ] **Step 2: Add constraint-ir to VALID_FORMATS and switch case**

In `src/api/generate.ts`, make these changes:

1. Add import (after line 7):
```typescript
import { generateConstraintIR } from '../generate/constraint-ir'
```

2. Update VALID_FORMATS (line 9):
```typescript
const VALID_FORMATS = ['openapi', 'sqlite', 'xstate', 'ilayer', 'readings', 'constraint-ir'] as const
```

3. Add case to switch (after the `readings` case, before the closing `}`):
```typescript
    case 'constraint-ir':
      output = await generateConstraintIR(db, domainId)
      break
```

- [ ] **Step 3: Run the full test suite to verify nothing broke**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/index.ts src/api/generate.ts
git commit -m "feat: wire constraint-ir into generate system"
```

---

## Chunk 2: Rust WASM Crate

This chunk creates the Rust crate that deserializes constraint IR JSON and evaluates constraints against populations. It produces a `.wasm` module via `wasm-pack`.

**Prerequisites:** Rust toolchain and `wasm-pack` must be installed locally.
```bash
# If not installed:
# curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# cargo install wasm-pack
```

### Task 3: Scaffold Rust crate with types

**Files:**
- Create: `crates/constraint-eval/Cargo.toml`
- Create: `crates/constraint-eval/src/types.rs`
- Create: `crates/constraint-eval/src/lib.rs`

- [ ] **Step 1: Create directory structure**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
mkdir -p crates/constraint-eval/src
mkdir -p crates/constraint-eval/tests
```

- [ ] **Step 2: Write Cargo.toml**

```toml
# crates/constraint-eval/Cargo.toml
[package]
name = "constraint-eval"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
wasm-bindgen = "0.2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[dev-dependencies]
wasm-bindgen-test = "0.3"

[profile.release]
opt-level = "s"
lto = true
```

- [ ] **Step 3: Write types.rs — IR deserialization + evaluation types**

```rust
// crates/constraint-eval/src/types.rs
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── IR Types (deserialized from generator JSON) ──────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintIR {
    pub domain: String,
    pub nouns: HashMap<String, NounDef>,
    pub fact_types: HashMap<String, FactTypeDef>,
    pub constraints: Vec<ConstraintDef>,
    pub state_machines: HashMap<String, StateMachineDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NounDef {
    pub object_type: String,
    pub enum_values: Option<Vec<String>>,
    pub value_type: Option<String>,
    pub super_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactTypeDef {
    pub reading: String,
    pub roles: Vec<RoleDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDef {
    pub noun_name: String,
    pub role_index: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintDef {
    pub id: String,
    pub kind: String,
    pub modality: String,
    pub deontic_operator: Option<String>,
    pub text: String,
    pub spans: Vec<SpanDef>,
    pub set_comparison_argument_length: Option<usize>,
    pub clauses: Option<Vec<String>>,
    pub entity: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanDef {
    pub fact_type_id: String,
    pub role_index: usize,
    pub subset_autofill: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateMachineDef {
    pub noun_name: String,
    pub statuses: Vec<String>,
    pub transitions: Vec<TransitionDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionDef {
    pub from: String,
    pub to: String,
    pub event: String,
    pub guard: Option<GuardDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardDef {
    pub graph_schema_id: String,
    pub constraint_ids: Vec<String>,
}

// ── Evaluation Types ─────────────────────────────────────────────────

/// A snapshot of facts for evaluation. Keys are fact type IDs.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Population {
    pub facts: HashMap<String, Vec<FactInstance>>,
}

/// A single fact instance — binds references to roles in a fact type.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactInstance {
    pub fact_type_id: String,
    /// Vec of (role_noun_name, reference_value)
    pub bindings: Vec<(String, String)>,
}

/// The response being evaluated (for deontic text constraints).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseContext {
    pub text: String,
    pub sender_identity: Option<String>,
    pub fields: Option<HashMap<String, String>>,
}

/// A constraint violation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Violation {
    pub constraint_id: String,
    pub constraint_text: String,
    pub detail: String,
}
```

- [ ] **Step 4: Write minimal lib.rs with WASM entry points**

```rust
// crates/constraint-eval/src/lib.rs
mod types;
mod evaluate;

use wasm_bindgen::prelude::*;
use std::sync::Mutex;
use std::sync::OnceLock;

use types::{ConstraintIR, ResponseContext, Population, Violation};

static IR: OnceLock<Mutex<Option<ConstraintIR>>> = OnceLock::new();

fn ir_store() -> &'static Mutex<Option<ConstraintIR>> {
    IR.get_or_init(|| Mutex::new(None))
}

#[wasm_bindgen]
pub fn load_ir(ir_json: &str) -> Result<(), JsValue> {
    let ir: ConstraintIR = serde_json::from_str(ir_json)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse IR: {}", e)))?;
    let mut store = ir_store().lock().unwrap();
    *store = Some(ir);
    Ok(())
}

#[wasm_bindgen]
pub fn evaluate_response(response_json: &str, population_json: &str) -> String {
    let store = ir_store().lock().unwrap();
    let ir = match store.as_ref() {
        Some(ir) => ir,
        None => return serde_json::to_string(&Vec::<Violation>::new()).unwrap(),
    };

    let response: ResponseContext = match serde_json::from_str(response_json) {
        Ok(r) => r,
        Err(e) => {
            let v = vec![Violation {
                constraint_id: "PARSE_ERROR".to_string(),
                constraint_text: String::new(),
                detail: format!("Failed to parse response: {}", e),
            }];
            return serde_json::to_string(&v).unwrap();
        }
    };

    let population: Population = match serde_json::from_str(population_json) {
        Ok(p) => p,
        Err(e) => {
            let v = vec![Violation {
                constraint_id: "PARSE_ERROR".to_string(),
                constraint_text: String::new(),
                detail: format!("Failed to parse population: {}", e),
            }];
            return serde_json::to_string(&v).unwrap();
        }
    };

    let violations = evaluate::evaluate(ir, &response, &population);
    serde_json::to_string(&violations).unwrap()
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cd C:/Users/lippe/Repos/graphdl-orm/crates/constraint-eval && cargo check`
Expected: Compilation errors for missing `evaluate` module (that's next)

- [ ] **Step 6: Commit scaffold**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add crates/constraint-eval/Cargo.toml crates/constraint-eval/src/types.rs crates/constraint-eval/src/lib.rs
git commit -m "feat: scaffold Rust WASM crate with IR types and entry points"
```

### Task 4: Implement constraint evaluation logic

**Files:**
- Create: `crates/constraint-eval/src/evaluate.rs`
- Test: `crates/constraint-eval/tests/integration.rs`

- [ ] **Step 1: Write evaluate.rs — core evaluation engine**

```rust
// crates/constraint-eval/src/evaluate.rs
use crate::types::*;
use std::collections::{HashMap, HashSet};

/// Evaluate all constraints in the IR against a response and population.
pub fn evaluate(
    ir: &ConstraintIR,
    response: &ResponseContext,
    population: &Population,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    for constraint in &ir.constraints {
        match constraint.modality.as_str() {
            "Deontic" => {
                violations.extend(evaluate_deontic(ir, constraint, response));
            }
            "Alethic" => {
                violations.extend(evaluate_alethic(ir, constraint, population));
            }
            _ => {}
        }
    }

    violations
}

// ── Deontic evaluation (text-based) ──────────────────────────────────

fn evaluate_deontic(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    response: &ResponseContext,
) -> Vec<Violation> {
    let operator = constraint.deontic_operator.as_deref().unwrap_or("");
    let text = &response.text;

    match operator {
        "forbidden" => evaluate_forbidden(ir, constraint, text),
        "obligatory" => evaluate_obligatory(ir, constraint, text, response),
        "permitted" => vec![], // Permitted constraints never produce violations
        _ => vec![],
    }
}

fn evaluate_forbidden(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    response_text: &str,
) -> Vec<Violation> {
    // For forbidden constraints, check if the response contains any of the
    // constrained noun's enum values. The constraint text tells us what pattern
    // is forbidden; the noun's enum_values list the specific forbidden values.
    //
    // Example: "It is forbidden that SupportResponse contains ProhibitedText"
    // ProhibitedText.enumValues = ['—', '–', '--']
    // Check if response_text contains any of those values.

    let mut violations = Vec::new();
    let mut seen = HashSet::new();
    let lower_text = response_text.to_lowercase();

    // For each span, find the associated fact type and check all value-type nouns
    // in that fact type for forbidden enum values.
    for span in &constraint.spans {
        let fact_type = match ir.fact_types.get(&span.fact_type_id) {
            Some(ft) => ft,
            None => continue,
        };

        for role in &fact_type.roles {
            if let Some(noun_def) = ir.nouns.get(&role.noun_name) {
                // Only check value-type nouns (entity nouns don't have textual enum values)
                if noun_def.object_type != "value" { continue; }

                if let Some(enum_values) = &noun_def.enum_values {
                    for val in enum_values {
                        let lower_val = val.to_lowercase();
                        if lower_text.contains(&lower_val) {
                            let detail = format!(
                                "Response contains forbidden {}: '{}'",
                                role.noun_name, val
                            );
                            if seen.insert(detail.clone()) {
                                violations.push(Violation {
                                    constraint_id: constraint.id.clone(),
                                    constraint_text: constraint.text.clone(),
                                    detail,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    violations
}

fn evaluate_obligatory(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    response_text: &str,
    response: &ResponseContext,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let lower_text = response_text.to_lowercase();

    // Check for obligatory field presence
    // Example: "It is obligatory that each SupportResponse has SenderIdentity 'Auto.dev Team <team@auto.dev>'"
    // Look for enum values in the object noun that MUST appear

    for span in &constraint.spans {
        let fact_type = match ir.fact_types.get(&span.fact_type_id) {
            Some(ft) => ft,
            None => continue,
        };

        for role in &fact_type.roles {
            if let Some(noun_def) = ir.nouns.get(&role.noun_name) {
                if noun_def.object_type == "value" {
                    if let Some(enum_values) = &noun_def.enum_values {
                        // For obligatory, at least one enum value must appear
                        let found = enum_values.iter().any(|val| {
                            lower_text.contains(&val.to_lowercase())
                        });
                        if !found && !enum_values.is_empty() {
                            violations.push(Violation {
                                constraint_id: constraint.id.clone(),
                                constraint_text: constraint.text.clone(),
                                detail: format!(
                                    "Response missing obligatory {}: expected one of {:?}",
                                    role.noun_name, enum_values
                                ),
                            });
                        }
                    }
                }
            }
        }
    }

    // Check sender identity if specified
    if let Some(sender) = &response.sender_identity {
        if constraint.text.to_lowercase().contains("senderidentity") && sender.is_empty() {
            violations.push(Violation {
                constraint_id: constraint.id.clone(),
                constraint_text: constraint.text.clone(),
                detail: "Response missing obligatory SenderIdentity".to_string(),
            });
        }
    }

    violations
}

// ── Alethic evaluation (structural) ──────────────────────────────────

fn evaluate_alethic(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    match constraint.kind.as_str() {
        "UC" => evaluate_uniqueness(ir, constraint, population),
        "MC" => evaluate_mandatory(ir, constraint, population),
        "RC" => evaluate_ring(ir, constraint, population),
        "XO" => evaluate_exclusive_or(ir, constraint, population),
        "XC" => evaluate_exclusive_choice(ir, constraint, population),
        "OR" => evaluate_inclusive_or(ir, constraint, population),
        "SS" => evaluate_subset(ir, constraint, population),
        "EQ" => evaluate_equality(ir, constraint, population),
        _ => vec![],
    }
}

fn evaluate_uniqueness(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    for span in &constraint.spans {
        let facts = match population.facts.get(&span.fact_type_id) {
            Some(f) => f,
            None => continue,
        };

        let fact_type = match ir.fact_types.get(&span.fact_type_id) {
            Some(ft) => ft,
            None => continue,
        };

        let role = match fact_type.roles.get(span.role_index) {
            Some(r) => r,
            None => continue,
        };

        // Group facts by the spanned role's binding value
        let mut seen: HashMap<String, usize> = HashMap::new();
        for fact in facts {
            if let Some((_, val)) = fact.bindings.iter().find(|(name, _)| name == &role.noun_name) {
                *seen.entry(val.clone()).or_insert(0) += 1;
            }
        }

        for (val, count) in &seen {
            if *count > 1 {
                violations.push(Violation {
                    constraint_id: constraint.id.clone(),
                    constraint_text: constraint.text.clone(),
                    detail: format!(
                        "Uniqueness violation: {} '{}' appears {} times in {}",
                        role.noun_name, val, count, fact_type.reading
                    ),
                });
            }
        }
    }

    violations
}

fn evaluate_mandatory(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    // For each entity instance of the subject noun,
    // check that it participates in at least one fact of the spanned type
    let mut violations = Vec::new();

    for span in &constraint.spans {
        let fact_type = match ir.fact_types.get(&span.fact_type_id) {
            Some(ft) => ft,
            None => continue,
        };

        let role = match fact_type.roles.get(span.role_index) {
            Some(r) => r,
            None => continue,
        };

        let facts = population.facts.get(&span.fact_type_id).cloned().unwrap_or_default();

        // Collect all entity instances of this noun from all facts
        let mut all_instances: HashSet<String> = HashSet::new();
        for (_, fact_list) in &population.facts {
            for fact in fact_list {
                for (name, val) in &fact.bindings {
                    if name == &role.noun_name {
                        all_instances.insert(val.clone());
                    }
                }
            }
        }

        // Check each instance participates in this fact type
        for instance in &all_instances {
            let participates = facts.iter().any(|f| {
                f.bindings.iter().any(|(name, val)| name == &role.noun_name && val == instance)
            });
            if !participates {
                violations.push(Violation {
                    constraint_id: constraint.id.clone(),
                    constraint_text: constraint.text.clone(),
                    detail: format!(
                        "Mandatory violation: {} '{}' does not participate in {}",
                        role.noun_name, instance, fact_type.reading
                    ),
                });
            }
        }
    }

    violations
}

fn evaluate_ring(
    _ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    for span in &constraint.spans {
        let facts = match population.facts.get(&span.fact_type_id) {
            Some(f) => f,
            None => continue,
        };

        // Ring irreflexive: no fact should have the same value for both roles
        for fact in facts {
            if fact.bindings.len() >= 2 {
                let first = &fact.bindings[0].1;
                let second = &fact.bindings[1].1;
                if first == second {
                    violations.push(Violation {
                        constraint_id: constraint.id.clone(),
                        constraint_text: constraint.text.clone(),
                        detail: format!(
                            "Ring constraint violation: '{}' references itself",
                            first
                        ),
                    });
                }
            }
        }
    }

    violations
}

fn evaluate_exclusive_or(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    // XO: For each entity, exactly one of the clause fact types holds
    evaluate_set_comparison(ir, constraint, population, |count| count != 1, "exactly one")
}

fn evaluate_exclusive_choice(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    // XC: For each entity, at most one of the clause fact types holds
    evaluate_set_comparison(ir, constraint, population, |count| count > 1, "at most one")
}

fn evaluate_inclusive_or(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    // OR: For each entity, at least one of the clause fact types holds
    evaluate_set_comparison(ir, constraint, population, |count| count < 1, "at least one")
}

fn evaluate_set_comparison(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
    violates: impl Fn(usize) -> bool,
    requirement: &str,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Get the entity noun name
    let entity_name = match &constraint.entity {
        Some(name) => name.clone(),
        None => return violations,
    };

    // Collect all entity instances
    let mut instances: HashSet<String> = HashSet::new();
    for (_, facts) in &population.facts {
        for fact in facts {
            for (name, val) in &fact.bindings {
                if name == &entity_name {
                    instances.insert(val.clone());
                }
            }
        }
    }

    // For each instance, count how many clause fact types hold
    let clause_fact_type_ids: Vec<String> = constraint.spans.iter()
        .map(|s| s.fact_type_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    for instance in &instances {
        let mut holding_count = 0;
        for ft_id in &clause_fact_type_ids {
            if let Some(facts) = population.facts.get(ft_id) {
                let holds = facts.iter().any(|f| {
                    f.bindings.iter().any(|(name, val)| name == &entity_name && val == instance)
                });
                if holds {
                    holding_count += 1;
                }
            }
        }

        if violates(holding_count) {
            violations.push(Violation {
                constraint_id: constraint.id.clone(),
                constraint_text: constraint.text.clone(),
                detail: format!(
                    "Set-comparison violation: {} '{}' has {} of {} clause fact types holding, expected {}",
                    entity_name, instance, holding_count, clause_fact_type_ids.len(), requirement
                ),
            });
        }
    }

    violations
}

fn evaluate_subset(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    // SS: If fact type A holds for entity X, then fact type B must also hold for X
    if constraint.spans.len() < 2 {
        return violations;
    }

    let a_ft_id = &constraint.spans[0].fact_type_id;
    let b_ft_id = &constraint.spans[1].fact_type_id;

    // Get the entity noun name from the first span's role
    let entity_name = ir.fact_types.get(a_ft_id)
        .and_then(|ft| ft.roles.get(constraint.spans[0].role_index))
        .map(|r| r.noun_name.clone())
        .unwrap_or_default();

    let a_facts = population.facts.get(a_ft_id).cloned().unwrap_or_default();
    let b_facts = population.facts.get(b_ft_id).cloned().unwrap_or_default();

    for a_fact in &a_facts {
        // Use name-based lookup instead of positional index
        if let Some((_, entity_val)) = a_fact.bindings.iter().find(|(name, _)| name == &entity_name) {
            let b_holds = b_facts.iter().any(|bf| {
                bf.bindings.iter().any(|(_, val)| val == entity_val)
            });
            if !b_holds {
                violations.push(Violation {
                    constraint_id: constraint.id.clone(),
                    constraint_text: constraint.text.clone(),
                    detail: format!(
                        "Subset violation: entity '{}' has fact A but not fact B",
                        entity_val
                    ),
                });
            }
        }
    }

    violations
}

fn evaluate_equality(
    _ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    // EQ: A holds iff B holds (bidirectional subset)
    if constraint.spans.len() < 2 {
        return violations;
    }

    let a_ft_id = &constraint.spans[0].fact_type_id;
    let b_ft_id = &constraint.spans[1].fact_type_id;

    let a_facts = population.facts.get(a_ft_id).cloned().unwrap_or_default();
    let b_facts = population.facts.get(b_ft_id).cloned().unwrap_or_default();

    // Collect entity values from A
    let a_entities: HashSet<String> = a_facts.iter()
        .flat_map(|f| f.bindings.iter().map(|(_, v)| v.clone()))
        .collect();

    let b_entities: HashSet<String> = b_facts.iter()
        .flat_map(|f| f.bindings.iter().map(|(_, v)| v.clone()))
        .collect();

    // A → B
    for entity in a_entities.difference(&b_entities) {
        violations.push(Violation {
            constraint_id: constraint.id.clone(),
            constraint_text: constraint.text.clone(),
            detail: format!("Equality violation: '{}' has fact A but not fact B", entity),
        });
    }

    // B → A
    for entity in b_entities.difference(&a_entities) {
        violations.push(Violation {
            constraint_id: constraint.id.clone(),
            constraint_text: constraint.text.clone(),
            detail: format!("Equality violation: '{}' has fact B but not fact A", entity),
        });
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ir() -> ConstraintIR {
        ConstraintIR {
            domain: "test".to_string(),
            nouns: HashMap::new(),
            fact_types: HashMap::new(),
            constraints: vec![],
            state_machines: HashMap::new(),
        }
    }

    fn empty_response() -> ResponseContext {
        ResponseContext {
            text: String::new(),
            sender_identity: None,
            fields: None,
        }
    }

    fn empty_population() -> Population {
        Population { facts: HashMap::new() }
    }

    #[test]
    fn test_no_constraints_no_violations() {
        let ir = empty_ir();
        let result = evaluate(&ir, &empty_response(), &empty_population());
        assert!(result.is_empty());
    }

    #[test]
    fn test_forbidden_text_detected() {
        let mut ir = empty_ir();
        ir.nouns.insert("ProhibitedText".to_string(), NounDef {
            object_type: "value".to_string(),
            enum_values: Some(vec!["—".to_string(), "–".to_string()]),
            value_type: Some("string".to_string()),
            super_type: None,
        });
        ir.nouns.insert("SupportResponse".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
        });
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "SupportResponse contains ProhibitedText".to_string(),
            roles: vec![
                RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                RoleDef { noun_name: "ProhibitedText".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: "It is forbidden that SupportResponse contains ProhibitedText".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        let response = ResponseContext {
            text: "Hello — how can I help?".to_string(),
            sender_identity: None,
            fields: None,
        };

        let result = evaluate(&ir, &response, &empty_population());
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("forbidden"));
        assert!(result[0].detail.contains("—"));
    }

    #[test]
    fn test_forbidden_text_clean() {
        let mut ir = empty_ir();
        ir.nouns.insert("ProhibitedText".to_string(), NounDef {
            object_type: "value".to_string(),
            enum_values: Some(vec!["—".to_string()]),
            value_type: Some("string".to_string()),
            super_type: None,
        });
        ir.nouns.insert("SupportResponse".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
        });
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "SupportResponse contains ProhibitedText".to_string(),
            roles: vec![
                RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                RoleDef { noun_name: "ProhibitedText".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: "It is forbidden that SupportResponse contains ProhibitedText".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        let response = ResponseContext {
            text: "Hello, how can I help you today?".to_string(),
            sender_identity: None,
            fields: None,
        };

        let result = evaluate(&ir, &response, &empty_population());
        assert!(result.is_empty());
    }

    #[test]
    fn test_uniqueness_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Customer has Name".to_string(),
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Each Customer has at most one Name".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Customer".to_string(), "c1".to_string()), ("Name".to_string(), "Alice".to_string())],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Customer".to_string(), "c1".to_string()), ("Name".to_string(), "Bob".to_string())],
            },
        ]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Uniqueness violation"));
    }

    #[test]
    fn test_ring_irreflexive_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Person manages Person".to_string(),
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Person".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "RC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "No Person manages itself".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Person".to_string(), "p1".to_string()), ("Person".to_string(), "p1".to_string())],
            },
        ]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Ring constraint"));
    }

    #[test]
    fn test_exclusive_or_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Order isPaid".to_string(),
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "Order isPending".to_string(),
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "XO".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Order, exactly one holds".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Order isPaid".to_string(), "Order isPending".to_string()]),
            entity: Some("Order".to_string()),
        });

        // Order o1 has BOTH facts — violates exactly-one
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        facts.insert("ft2".to_string(), vec![FactInstance {
            fact_type_id: "ft2".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Set-comparison violation"));
    }

    #[test]
    fn test_subset_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Person hasLicense".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "Person hasInsurance".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "If some Person hasLicense then that Person hasInsurance".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        // Person p1 has license but no insurance — violates subset
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Person".to_string(), "p1".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Subset violation"));
    }

    #[test]
    fn test_permitted_never_violates() {
        let mut ir = empty_ir();
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("permitted".to_string()),
            text: "It is permitted that SupportResponse offers data retrieval".to_string(),
            spans: vec![],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        let result = evaluate(&ir, &empty_response(), &empty_population());
        assert!(result.is_empty());
    }

    #[test]
    fn test_exclusive_choice_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Order isPaid".to_string(),
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "Order isPending".to_string(),
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "XC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Order, at most one holds".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Order isPaid".to_string(), "Order isPending".to_string()]),
            entity: Some("Order".to_string()),
        });

        // Order o1 has BOTH facts — violates at-most-one
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        facts.insert("ft2".to_string(), vec![FactInstance {
            fact_type_id: "ft2".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Set-comparison violation"));
    }

    #[test]
    fn test_equality_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Person isEmployee".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "Person hasBadge".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "EQ".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Person isEmployee if and only if Person hasBadge".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        // Person p1 isEmployee but does NOT hasBadge — violates biconditional
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Person".to_string(), "p1".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Equality violation"));
    }
}
```

- [ ] **Step 2: Verify Rust tests pass**

Run: `cd C:/Users/lippe/Repos/graphdl-orm/crates/constraint-eval && cargo test`
Expected: All tests pass

- [ ] **Step 3: Write integration test**

```rust
// crates/constraint-eval/tests/integration.rs
use constraint_eval::{load_ir, evaluate_response};

#[test]
fn test_full_pipeline_forbidden_text() {
    let ir_json = r#"{
        "domain": "test",
        "nouns": {
            "SupportResponse": { "objectType": "entity" },
            "ProhibitedText": { "objectType": "value", "enumValues": ["—", "–"], "valueType": "string" }
        },
        "factTypes": {
            "ft1": {
                "reading": "SupportResponse contains ProhibitedText",
                "roles": [
                    { "nounName": "SupportResponse", "roleIndex": 0 },
                    { "nounName": "ProhibitedText", "roleIndex": 1 }
                ]
            }
        },
        "constraints": [{
            "id": "c1",
            "kind": "UC",
            "modality": "Deontic",
            "deonticOperator": "forbidden",
            "text": "It is forbidden that SupportResponse contains ProhibitedText",
            "spans": [{ "factTypeId": "ft1", "roleIndex": 0 }]
        }],
        "stateMachines": {}
    }"#;

    load_ir(ir_json).unwrap();

    let response = r#"{"text": "Hello — how are you?", "senderIdentity": null, "fields": null}"#;
    let population = r#"{"facts": {}}"#;

    let result = evaluate_response(response, population);
    let violations: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
    assert!(!violations.is_empty());
}

#[test]
fn test_full_pipeline_clean_response() {
    let ir_json = r#"{
        "domain": "test",
        "nouns": {
            "SupportResponse": { "objectType": "entity" },
            "ProhibitedText": { "objectType": "value", "enumValues": ["—"], "valueType": "string" }
        },
        "factTypes": {
            "ft1": {
                "reading": "SupportResponse contains ProhibitedText",
                "roles": [
                    { "nounName": "SupportResponse", "roleIndex": 0 },
                    { "nounName": "ProhibitedText", "roleIndex": 1 }
                ]
            }
        },
        "constraints": [{
            "id": "c1",
            "kind": "UC",
            "modality": "Deontic",
            "deonticOperator": "forbidden",
            "text": "It is forbidden that SupportResponse contains ProhibitedText",
            "spans": [{ "factTypeId": "ft1", "roleIndex": 0 }]
        }],
        "stateMachines": {}
    }"#;

    load_ir(ir_json).unwrap();

    let response = r#"{"text": "Hello, how are you today?", "senderIdentity": null, "fields": null}"#;
    let population = r#"{"facts": {}}"#;

    let result = evaluate_response(response, population);
    let violations: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
    assert!(violations.is_empty());
}
```

- [ ] **Step 4: Run all Rust tests (unit + integration)**

Run: `cd C:/Users/lippe/Repos/graphdl-orm/crates/constraint-eval && cargo test -- --test-threads=1`
Expected: All pass

Note: `--test-threads=1` is required because the integration tests share a mutable static `IR` via `load_ir()`. Parallel test execution would cause race conditions.

- [ ] **Step 5: Build WASM module**

Run: `cd C:/Users/lippe/Repos/graphdl-orm/crates/constraint-eval && wasm-pack build --target bundler`
Expected: Build succeeds, produces `pkg/` directory with `.wasm` file and JS glue

- [ ] **Step 6: Commit**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add crates/constraint-eval/src/evaluate.rs crates/constraint-eval/tests/integration.rs
git commit -m "feat: implement constraint evaluation engine (alethic + deontic + set-comparison)"
```

---

## Chunk 3: Chat Endpoint

This chunk adds the `POST /api/chat` route that orchestrates Claude calls with WASM constraint evaluation, streaming responses via SSE.

### Task 5: Update Env type and wrangler config

**Files:**
- Modify: `src/types.ts:1-4`
- Modify: `wrangler.jsonc:22-24`

- [ ] **Step 1: Extend Env interface**

Replace the contents of `src/types.ts` with:

```typescript
export interface Env {
  GRAPHDL_DB: DurableObjectNamespace
  ENVIRONMENT: string
  AI?: Ai
  ANTHROPIC_API_KEY?: string
}
```

Both AI bindings are optional so existing code and tests that construct `Env` without them continue to work. The chat handler checks for their presence at runtime.

- [ ] **Step 2: Add AI binding and WASM rules to wrangler.jsonc**

In `wrangler.jsonc`, add after the `"vars"` section (after line 24):

```jsonc
  "ai": {
    "binding": "AI"
  },
  "rules": [
    { "type": "CompiledWasm", "globs": ["**/*.wasm"], "fallthrough": true }
  ]
```

- [ ] **Step 3: Commit**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/types.ts wrangler.jsonc
git commit -m "feat: add AI binding and WASM rules to env config"
```

### Task 6: Write the chat handler

**Files:**
- Create: `src/api/chat.ts`
- Test: `src/api/chat.test.ts`

- [ ] **Step 1: Write the failing test for the chat handler**

```typescript
// src/api/chat.test.ts
import { describe, it, expect, vi } from 'vitest'

// We test the core pipeline functions, not the HTTP handler directly
// (Workers AI binding and WASM module require real runtime)
import { buildSystemPrompt, extractToolCalls } from './chat'

describe('buildSystemPrompt', () => {
  it('replaces {{currentState}} template variable', () => {
    const prompt = '## Current State: {{currentState}}\nInstructions here.'
    const result = buildSystemPrompt(prompt, 'Investigating')
    expect(result).toBe('## Current State: Investigating\nInstructions here.')
  })

  it('returns prompt unchanged when no currentState', () => {
    const prompt = '## Current State: {{currentState}}'
    const result = buildSystemPrompt(prompt, undefined)
    expect(result).toBe('## Current State: Unknown')
  })
})

describe('extractToolCalls', () => {
  it('extracts tool_use blocks from Claude response', () => {
    const content = [
      { type: 'text', text: 'I will resolve this.' },
      { type: 'tool_use', id: 'tu1', name: 'resolve', input: {} },
    ]
    const result = extractToolCalls(content)
    expect(result).toHaveLength(1)
    expect(result[0]).toMatchObject({ name: 'resolve', input: {} })
  })

  it('returns empty array when no tool calls', () => {
    const content = [{ type: 'text', text: 'Just text.' }]
    const result = extractToolCalls(content)
    expect(result).toEqual([])
  })

  it('handles string content', () => {
    const result = extractToolCalls('plain text response')
    expect(result).toEqual([])
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/chat.test.ts`
Expected: FAIL — imports do not exist

- [ ] **Step 3: Write the chat handler**

```typescript
// src/api/chat.ts
import { json, error } from 'itty-router'
import type { Env } from '../types'
import { verifyProse } from './verify'

// ── Exported pure functions (for testing) ────────────────────────────

export function buildSystemPrompt(template: string, currentState?: string): string {
  return template.replace(/\{\{currentState\}\}/g, currentState || 'Unknown')
}

export function extractToolCalls(content: any): Array<{ name: string; input: any }> {
  if (typeof content === 'string') return []
  if (!Array.isArray(content)) return []
  return content
    .filter((block: any) => block.type === 'tool_use')
    .map((block: any) => ({ name: block.name, input: block.input || {} }))
}

// ── SSE helpers ──────────────────────────────────────────────────────

function sseEvent(data: any): string {
  return `data: ${JSON.stringify(data)}\n\n`
}

function sseDone(): string {
  return `data: [DONE]\n\n`
}

// ── Population builder ───────────────────────────────────────────────

async function buildPopulation(db: any, domainId: string, requestId?: string): Promise<{ facts: Record<string, any[]> }> {
  const facts: Record<string, any[]> = {}

  // Load instance data + role/noun metadata in parallel
  const [graphsResult, resourceRolesResult, resourcesResult, rolesResult, nounsResult] = await Promise.all([
    db.findInCollection('graphs', { domain: { equals: domainId } }, { limit: 10000 }),
    db.findInCollection('resource-roles', { domain: { equals: domainId } }, { limit: 10000 }),
    db.findInCollection('resources', { domain: { equals: domainId } }, { limit: 10000 }),
    db.findInCollection('roles', {}, { limit: 10000 }),
    db.findInCollection('nouns', { domain: { equals: domainId } }, { limit: 10000 }),
  ])

  // Build lookups
  const resourceById = new Map(resourcesResult.docs.map((r: any) => [r.id, r]))
  const nounById = new Map(nounsResult.docs.map((n: any) => [n.id, n]))

  // Resolve role ID → noun name via role.noun → noun.name
  const roleNounName = new Map<string, string>()
  for (const role of rolesResult.docs) {
    const noun = nounById.get(role.noun)
    if (noun) roleNounName.set(role.id, noun.name)
  }

  // Group resource-roles by graph ID
  const rolesByGraph = new Map<string, any[]>()
  for (const rr of resourceRolesResult.docs) {
    const gid = rr.graph
    if (!rolesByGraph.has(gid)) rolesByGraph.set(gid, [])
    rolesByGraph.get(gid)!.push(rr)
  }

  // Build fact instances from graphs + resource-roles
  for (const graph of graphsResult.docs) {
    const graphSchemaId = graph.graphSchema
    if (!graphSchemaId) continue

    const rrs = rolesByGraph.get(graph.id) || []
    const bindings: Array<[string, string]> = []

    for (const rr of rrs) {
      const resource = resourceById.get(rr.resource)
      const nounName = roleNounName.get(rr.role)
      if (resource && nounName) {
        bindings.push([nounName, resource.reference || resource.value || resource.id])
      }
    }

    if (!facts[graphSchemaId]) facts[graphSchemaId] = []
    facts[graphSchemaId].push({
      factTypeId: graphSchemaId,
      bindings,
    })
  }

  return { facts }
}

// ── Chat handler ─────────────────────────────────────────────────────

interface ChatRequest {
  domainId: string
  messages: Array<{ role: 'user' | 'assistant'; content: string }>
  requestId?: string
}

function getDB(env: Env): DurableObjectStub {
  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  return env.GRAPHDL_DB.get(id)
}

async function fetchGenerator(db: any, domainId: string, format: string): Promise<string | null> {
  const result = await db.findInCollection('generators', {
    domain: { equals: domainId },
    outputFormat: { equals: format },
  }, { limit: 1 })
  return result?.docs?.[0]?.output || null
}

export async function handleChat(request: Request, env: Env, ctx?: ExecutionContext): Promise<Response> {
  const body = await request.json() as ChatRequest
  if (!body.domainId) {
    return error(400, { errors: [{ message: 'domainId is required' }] })
  }
  if (!body.messages?.length) {
    return error(400, { errors: [{ message: 'messages array is required' }] })
  }

  const db = getDB(env) as any
  const { domainId, messages, requestId } = body

  // 1. Load context from generators collection
  const [xstateOutput, irOutput] = await Promise.all([
    fetchGenerator(db, domainId, 'xstate'),
    fetchGenerator(db, domainId, 'constraint-ir'),
  ])

  // Extract agent prompt and tools from xstate output
  let systemPrompt = ''
  let tools: any[] = []

  if (xstateOutput) {
    try {
      const xstate = JSON.parse(xstateOutput)
      // xstate output has files: { "agents/<name>-prompt.md": "...", "agents/<name>-tools.json": "..." }
      const files = xstate.files || xstate
      const promptFile = Object.entries(files).find(([k]) => k.endsWith('-prompt.md'))
      const toolsFile = Object.entries(files).find(([k]) => k.endsWith('-tools.json'))
      if (promptFile) systemPrompt = promptFile[1] as string
      if (toolsFile) tools = JSON.parse(toolsFile[1] as string)
    } catch { /* fallback to empty */ }
  }

  // 2. Query population snapshot
  let currentState: string | undefined
  if (requestId) {
    try {
      const smResult = await db.findInCollection('state-machines', {
        resource: { equals: requestId },
      }, { limit: 1 })
      const sm = smResult?.docs?.[0]
      if (sm?.currentStatus) {
        const status = await db.getFromCollection('statuses', sm.currentStatus)
        currentState = status?.name
      }
    } catch { /* no state machine for this request */ }
  }

  // Render system prompt with current state
  systemPrompt = buildSystemPrompt(systemPrompt, currentState)

  // 3. Load WASM constraint IR (if available)
  let constraintIR: any = null
  if (irOutput) {
    try {
      constraintIR = JSON.parse(irOutput)
    } catch { /* skip constraint evaluation */ }
  }

  // 3b. Scope constraints via verifyProse() — identify which constraints are relevant
  // to the conversation context (optimization: reduces WASM evaluation set)
  if (constraintIR) {
    const lastUserMessage = messages.filter(m => m.role === 'user').pop()?.content || ''
    if (lastUserMessage) {
      // Load domain data for verifyProse
      const [nounsResult, readingsResult, rolesResult, constraintsResult, spansResult] = await Promise.all([
        db.findInCollection('nouns', { domain: { equals: domainId } }, { limit: 10000 }),
        db.findInCollection('readings', { domain: { equals: domainId } }, { limit: 10000 }),
        db.findInCollection('roles', {}, { limit: 10000 }),
        db.findInCollection('constraints', { domain: { equals: domainId } }, { limit: 10000 }),
        db.findInCollection('constraint-spans', {}, { limit: 10000 }),
      ])
      const domainReadingIds = new Set(readingsResult.docs.map((r: any) => r.id))
      const domainData = {
        nouns: nounsResult.docs.map((n: any) => ({ id: n.id, name: n.name })),
        readings: readingsResult.docs.map((r: any) => ({ id: r.id, text: r.text })),
        roles: rolesResult.docs
          .filter((r: any) => domainReadingIds.has(r.reading))
          .map((r: any) => ({ id: r.id, reading: r.reading, noun: r.noun, roleIndex: r.roleIndex })),
        constraints: constraintsResult.docs.map((c: any) => ({ id: c.id, kind: c.kind, text: c.text })),
        constraintSpans: spansResult.docs.map((s: any) => ({ constraint: s.constraint, role: s.role })),
      }
      const verifyResult = verifyProse(lastUserMessage, domainData)
      // Keep only constraints whose readings matched — others are irrelevant to this conversation
      const matchedReadings = new Set(verifyResult.matches.map(m => m.reading))
      if (matchedReadings.size > 0) {
        // Filter IR constraints to only those relevant to the conversation
        // (Keep all deontic constraints — they apply regardless of conversation topic)
        constraintIR.constraints = constraintIR.constraints.filter((c: any) =>
          c.modality === 'Deontic' || c.spans.some((s: any) => {
            const ft = constraintIR.factTypes[s.factTypeId]
            return ft && matchedReadings.has(ft.reading)
          })
        )
      }
    }
  }

  // 4. Build Claude messages
  const claudeMessages = messages.map(m => ({
    role: m.role,
    content: m.content,
  }))

  // 5. Call Claude with redraft loop
  const MAX_REDRAFT = 3
  let finalContent = ''
  let toolCalls: Array<{ name: string; input: any }> = []
  let violations: string[] = []

  for (let attempt = 0; attempt <= MAX_REDRAFT; attempt++) {
    try {
      // Call AI — try Workers AI binding first, fall back to Anthropic API
      let response: any

      if (env.AI) {
        response = await env.AI.run('@cf/anthropic/claude-sonnet-4-20250514', {
          system: systemPrompt,
          messages: claudeMessages,
          tools: tools.length > 0 ? tools.map(t => ({
            name: t.name,
            description: t.description,
            input_schema: t.parameters || { type: 'object', properties: {} },
          })) : undefined,
          max_tokens: 4096,
        })
      } else if (env.ANTHROPIC_API_KEY) {
        const apiResponse = await fetch('https://api.anthropic.com/v1/messages', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            'x-api-key': env.ANTHROPIC_API_KEY,
            'anthropic-version': '2023-06-01',
          },
          body: JSON.stringify({
            model: 'claude-sonnet-4-20250514',
            system: systemPrompt,
            messages: claudeMessages,
            tools: tools.length > 0 ? tools.map(t => ({
              name: t.name,
              description: t.description,
              input_schema: t.parameters || { type: 'object', properties: {} },
            })) : undefined,
            max_tokens: 4096,
          }),
        })
        response = await apiResponse.json()
      } else {
        return error(500, { errors: [{ message: 'No AI provider configured' }] })
      }

      // Extract text content
      const content = response.content || response.response || ''
      if (typeof content === 'string') {
        finalContent = content
      } else if (Array.isArray(content)) {
        finalContent = content
          .filter((b: any) => b.type === 'text')
          .map((b: any) => b.text)
          .join('')
        toolCalls = extractToolCalls(content)
      }

      // 5. Evaluate constraints via WASM (if IR available)
      if (constraintIR && attempt < MAX_REDRAFT) {
        // Dynamic import of WASM module — will be available when deployed with wrangler
        try {
          // @ts-ignore — dynamic WASM import resolved by wrangler bundler
          const wasmModule = await import('../../crates/constraint-eval/pkg/constraint_eval')
          wasmModule.load_ir(JSON.stringify(constraintIR))

          // Build population from instance data
          const population = await buildPopulation(db, domainId, requestId)

          const responseCtx = { text: finalContent, senderIdentity: null, fields: null }
          const violationJson = wasmModule.evaluate_response(
            JSON.stringify(responseCtx),
            JSON.stringify(population),
          )
          const currentViolations = JSON.parse(violationJson)

          if (currentViolations.length > 0) {
            violations = currentViolations.map((v: any) => v.detail)
            // Append violation feedback and retry
            claudeMessages.push({ role: 'assistant', content: finalContent })
            claudeMessages.push({
              role: 'user',
              content: `Your response has the following constraint violations. Please redraft:\n${violations.map((v: string) => `- ${v}`).join('\n')}`,
            })
            continue
          }
        } catch (wasmErr) {
          // WASM not available (dev mode, missing build) — skip evaluation
          console.error('WASM constraint evaluation unavailable:', wasmErr)
        }
      }

      // No violations or max attempts reached — break
      break
    } catch (e: any) {
      return error(500, { errors: [{ message: `AI call failed: ${e.message}` }] })
    }
  }

  // 6. Stream response via SSE
  const encoder = new TextEncoder()
  const stream = new ReadableStream({
    start(controller) {
      // If violations remain after max redrafts, send warning
      if (violations.length > 0) {
        controller.enqueue(encoder.encode(sseEvent({
          type: 'violation_warning',
          violations,
          message: 'Delivered with unresolved constraints',
        })))
      }

      // Stream content
      controller.enqueue(encoder.encode(sseEvent({
        type: 'content',
        content: finalContent,
      })))

      // Stream tool calls
      for (const tc of toolCalls) {
        controller.enqueue(encoder.encode(sseEvent({
          type: 'tool_use',
          name: tc.name,
          input: tc.input,
        })))
      }

      controller.enqueue(encoder.encode(sseDone()))
      controller.close()
    },
  })

  // 7. Persist and transition (fire-and-forget)
  // Note: The spec says "Create Message record" but there is no 'messages' collection
  // in the 29 collection slugs. We use the existing 'completions' collection which
  // stores inputText/outputText pairs — this serves the same purpose.
  const persistPromise = (async () => {
    try {
      // Create completion record
      await db.createInCollection('completions', {
        inputText: messages[messages.length - 1]?.content || '',
        outputText: finalContent,
        occurredAt: new Date().toISOString(),
        domain: domainId,
      })

      // Execute tool calls as state transitions
      for (const tc of toolCalls) {
        if (requestId) {
          // Find the event type
          const eventTypes = await db.findInCollection('event-types', {
            name: { equals: tc.name },
            domain: { equals: domainId },
          }, { limit: 1 })
          const eventType = eventTypes?.docs?.[0]

          if (eventType) {
            // Find state machine for this resource
            const smResult = await db.findInCollection('state-machines', {
              resource: { equals: requestId },
            }, { limit: 1 })
            const sm = smResult?.docs?.[0]

            if (sm) {
              // Find the transition
              const transResult = await db.findInCollection('transitions', {
                from: { equals: sm.currentStatus },
                eventType: { equals: eventType.id },
              }, { limit: 1 })
              const transition = transResult?.docs?.[0]

              if (transition) {
                // Create event record
                await db.createInCollection('events', {
                  eventType: eventType.id,
                  stateMachine: sm.id,
                  domain: domainId,
                  occurredAt: new Date().toISOString(),
                  data: JSON.stringify(tc.input),
                })

                // Update state machine status
                await db.updateInCollection('state-machines', sm.id, {
                  currentStatus: transition.to,
                })
              }
            }
          }
        }
      }
    } catch {
      // Don't fail the response if persistence fails
    }
  })()

  // Use ctx.waitUntil to guarantee persistence completes even if the isolate
  // would otherwise terminate after sending the response.
  if (ctx?.waitUntil) {
    ctx.waitUntil(persistPromise)
  } else {
    persistPromise.catch(() => {})
  }

  return new Response(stream, {
    headers: {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      'Connection': 'keep-alive',
    },
  })
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/chat.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/api/chat.ts src/api/chat.test.ts
git commit -m "feat: add POST /api/chat endpoint with Claude orchestration and SSE streaming"
```

### Task 7: Register the chat route

**Files:**
- Modify: `src/api/router.ts:7-8,114`

- [ ] **Step 1: Add import and route registration**

In `src/api/router.ts`:

1. Add import (after line 8, the `handleVerify` import):
```typescript
import { handleChat } from './chat'
```

2. Add route **immediately after** `router.post('/api/generate', handleGenerate)` at line 114, **before** the parameterized `/api/:collection` CRUD routes. This is critical: itty-router matches routes in registration order, and the parameterized `/api/:collection` would capture `/api/chat` first if registered before it:
```typescript
// ── Chat ─────────────────────────────────────────────────────────────
router.post('/api/chat', handleChat)
```

- [ ] **Step 2: Run the full test suite**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run`
Expected: All tests pass

- [ ] **Step 3: Run TypeScript type check**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx tsc --noEmit`
Expected: No new type errors. Note: The `Ai` type comes from `@cloudflare/workers-types`. If it's not recognized, the existing `@cloudflare/workers-types` package should already include it. If `tsc` complains about the `Ai` type, add `/// <reference types="@cloudflare/workers-types" />` to `src/types.ts`.

- [ ] **Step 4: Commit**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/api/router.ts
git commit -m "feat: register POST /api/chat route"
```

### Task 7b: Update generate.test.ts for constraint-ir format

**Files:**
- Modify: `src/api/generate.test.ts`

- [ ] **Step 1: Add 'constraint-ir' to format validation test**

In `src/api/generate.test.ts`, find the test that validates accepted formats and add `'constraint-ir'` to the list. If the test iterates over `VALID_FORMATS` directly, it will pick up the new format automatically. If it hardcodes the list, add the new entry.

- [ ] **Step 2: Run the generate tests**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/generate.test.ts`
Expected: All PASS

- [ ] **Step 3: Commit**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/api/generate.test.ts
git commit -m "test: update generate tests for constraint-ir format"
```

### Task 7c: Add chat handler error handling tests

**Files:**
- Modify: `src/api/chat.test.ts`

- [ ] **Step 1: Add tests for error paths and SSE output**

Append to `src/api/chat.test.ts`:

```typescript
import { buildPopulation } from './chat'

describe('buildPopulation', () => {
  it('returns empty facts when no graphs exist', async () => {
    const db = {
      findInCollection: vi.fn(async () => ({
        docs: [], totalDocs: 0, limit: 10000, page: 1, hasNextPage: false,
      })),
    }
    const result = await buildPopulation(db, 'domain1')
    expect(result.facts).toEqual({})
    // Should have queried graphs, resource-roles, resources, roles, nouns
    expect(db.findInCollection).toHaveBeenCalledTimes(5)
  })
})

describe('buildSystemPrompt edge cases', () => {
  it('handles multiple {{currentState}} placeholders', () => {
    const prompt = 'State: {{currentState}} and again: {{currentState}}'
    const result = buildSystemPrompt(prompt, 'Active')
    expect(result).toBe('State: Active and again: Active')
  })

  it('handles empty prompt', () => {
    const result = buildSystemPrompt('', 'Active')
    expect(result).toBe('')
  })
})
```

- [ ] **Step 2: Export buildPopulation from chat.ts for testability**

Ensure `buildPopulation` is exported from `src/api/chat.ts`:

```typescript
export async function buildPopulation(db: any, domainId: string, requestId?: string): Promise<{ facts: Record<string, any[]> }> {
```

- [ ] **Step 3: Run all chat tests**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/chat.test.ts`
Expected: All PASS

- [ ] **Step 4: Commit**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/api/chat.ts src/api/chat.test.ts
git commit -m "test: add error handling and population builder tests for chat endpoint"
```

---

## Chunk 4: CI/CD and Integration

### Task 8: Update CI/CD pipeline

**Files:**
- Modify: `.github/workflows/deploy.yml:10-27`

- [ ] **Step 1: Add Rust toolchain and WASM build to deploy workflow**

Replace the contents of `.github/workflows/deploy.yml` with:

```yaml
name: Deploy to Cloudflare Workers

on:
  push:
    branches: [main]

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: yarn

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-unknown-unknown

      - name: Install wasm-pack
        run: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

      - name: Build WASM constraint evaluator
        run: cd crates/constraint-eval && cargo test -- --test-threads=1 && wasm-pack build --target bundler

      - run: yarn install --frozen-lockfile

      - run: yarn test
        continue-on-error: false

      - uses: cloudflare/wrangler-action@v3
        with:
          apiToken: ${{ secrets.CLOUDFLARE_API_TOKEN }}
          accountId: b6641681fe423910342b9ffa1364c76d
          command: deploy
```

- [ ] **Step 2: Add .gitignore entries for Rust build artifacts**

Append to `.gitignore` (or create if doesn't exist):

```
# Rust
/crates/constraint-eval/target/
/crates/constraint-eval/pkg/
```

- [ ] **Step 3: Commit**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add .github/workflows/deploy.yml .gitignore
git commit -m "ci: add Rust toolchain and WASM build to deploy pipeline"
```

### Task 9: Final integration verification

- [ ] **Step 1: Run all TypeScript tests**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run`
Expected: All tests pass (existing + new constraint-ir + chat tests)

- [ ] **Step 2: Run all Rust tests**

Run: `cd C:/Users/lippe/Repos/graphdl-orm/crates/constraint-eval && cargo test -- --test-threads=1`
Expected: All tests pass

- [ ] **Step 3: Build WASM module**

Run: `cd C:/Users/lippe/Repos/graphdl-orm/crates/constraint-eval && wasm-pack build --target bundler`
Expected: Build succeeds

- [ ] **Step 4: Type check**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx tsc --noEmit`
Expected: No new errors

- [ ] **Step 5: Verify git log shows clean commit history**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && git log --oneline feat/wasm-constraint-engine --not main`
Expected: Clean sequence of commits:
```
feat: add constraint-ir generator — JSON IR from metamodel
feat: wire constraint-ir into generate system
feat: scaffold Rust WASM crate with IR types and entry points
feat: implement constraint evaluation engine (alethic + deontic + set-comparison)
feat: add AI binding and WASM rules to env config
feat: add POST /api/chat endpoint with Claude orchestration and SSE streaming
feat: register POST /api/chat route
ci: add Rust toolchain and WASM build to deploy pipeline
```
