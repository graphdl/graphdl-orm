# Functional Domain Model Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace ad-hoc per-generator query logic with a lazy, cached DomainModel on the DO. Generators become rendering functions passed into the model. Add a new mdxui generator as proof of the architecture.

**Architecture:** DomainModel wraps SqlStorage on the DO, provides typed cached accessors (nouns, factTypes, constraints, stateMachines, readings, constraintSpans). Two generator styles: walker generators use `render()` traversal, direct generators access DomainModel methods. A new `generate()` RPC method on the DO replaces per-collection RPC calls from the Worker. Auto-invalidation in DO write methods keeps caches fresh.

**Tech Stack:** TypeScript, Cloudflare Workers + Durable Objects, SQLite (via SqlStorage), Vitest, mdxui (React/MDX components)

**Spec:** `docs/superpowers/specs/2026-03-13-functional-domain-model-design.md`

---

## Chunk 1: Foundation — Types, DomainModel, DO Integration

### Task 1: Model Types

**Files:**
- Create: `src/model/types.ts`
- Test: `src/model/types.test.ts`

- [ ] **Step 1: Write type validation test**

```typescript
// src/model/types.test.ts
import { describe, it, expect } from 'vitest'
import type { NounDef, FactTypeDef, RoleDef, ConstraintDef, SpanDef, StateMachineDef, StatusDef, TransitionDef, VerbDef, ReadingDef } from './types'

describe('model types', () => {
  it('NounDef represents entity nouns', () => {
    const noun: NounDef = {
      id: 'n1', name: 'Customer', objectType: 'entity', domainId: 'd1',
      plural: 'customers', permissions: ['list', 'read', 'create', 'update', 'delete'],
    }
    expect(noun.objectType).toBe('entity')
  })

  it('NounDef represents value nouns with validation', () => {
    const noun: NounDef = {
      id: 'n2', name: 'Email', objectType: 'value', domainId: 'd1',
      valueType: 'string', format: 'email', minLength: 5, maxLength: 255,
    }
    expect(noun.valueType).toBe('string')
  })

  it('NounDef supports enum values as parsed string[]', () => {
    const noun: NounDef = {
      id: 'n3', name: 'Priority', objectType: 'value', domainId: 'd1',
      valueType: 'string', enumValues: ['low', 'medium', 'high'],
    }
    expect(noun.enumValues).toEqual(['low', 'medium', 'high'])
  })

  it('FactTypeDef carries reading, roles, and arity', () => {
    const customer: NounDef = { id: 'n1', name: 'Customer', objectType: 'entity', domainId: 'd1' }
    const name: NounDef = { id: 'n2', name: 'Name', objectType: 'value', domainId: 'd1', valueType: 'string' }
    const ft: FactTypeDef = {
      id: 'gs1', name: 'CustomerHasName', reading: 'Customer has Name',
      roles: [
        { id: 'r1', nounName: 'Customer', nounDef: customer, roleIndex: 0 },
        { id: 'r2', nounName: 'Name', nounDef: name, roleIndex: 1 },
      ],
      arity: 2,
    }
    expect(ft.arity).toBe(2)
    expect(ft.roles[0].nounDef.name).toBe('Customer')
  })

  it('ConstraintDef supports all 8 kinds with spans', () => {
    const c: ConstraintDef = {
      id: 'c1', kind: 'UC', modality: 'Alethic', text: 'Each Customer has at most one Name',
      spans: [{ factTypeId: 'gs1', roleIndex: 1 }],
    }
    expect(c.kind).toBe('UC')
    expect(c.spans).toHaveLength(1)
  })

  it('ConstraintDef supports deontic constraints', () => {
    const c: ConstraintDef = {
      id: 'c2', kind: 'MC', modality: 'Deontic', deonticOperator: 'obligatory',
      text: 'It is obligatory that each Order has at least one Item',
      spans: [{ factTypeId: 'gs2', roleIndex: 0 }],
    }
    expect(c.deonticOperator).toBe('obligatory')
  })

  it('StateMachineDef carries resolved statuses and transitions with verbs', () => {
    const noun: NounDef = { id: 'n1', name: 'Order', objectType: 'entity', domainId: 'd1' }
    const sm: StateMachineDef = {
      id: 'sm1', nounName: 'Order', nounDef: noun,
      statuses: [{ id: 's1', name: 'Draft' }, { id: 's2', name: 'Pending' }],
      transitions: [{
        from: 'Draft', to: 'Pending', event: 'Submit', eventTypeId: 'et1',
        verb: { id: 'v1', name: 'submitOrder', func: { callbackUrl: '/api/submit', httpMethod: 'POST' } },
      }],
    }
    expect(sm.transitions[0].verb?.func?.callbackUrl).toBe('/api/submit')
    expect(sm.transitions[0].verb?.func?.httpMethod).toBe('POST')
  })
})
```

- [ ] **Step 2: Run test — expect compile errors (types don't exist yet)**

Run: `npx vitest run src/model/types.test.ts`
Expected: FAIL — cannot find module `./types`

- [ ] **Step 3: Create types.ts with all type definitions**

Create `src/model/types.ts` with all interfaces from the spec (§1 Types section). Copy the full type definitions from the spec — `NounDef`, `FactTypeDef`, `RoleDef`, `ConstraintDef`, `SpanDef`, `StateMachineDef`, `StatusDef`, `TransitionDef`, `VerbDef`, `ReadingDef`. Export all interfaces.

Key details from spec:
- `NounDef.enumValues` is `string[]` (DomainModel pre-parses from JSON string in DB)
- `NounDef.superType` is `NounDef | string` (resolved when possible)
- `NounDef.referenceScheme` is `NounDef[]` — NOT a DB column. DomainModel derives this from UC constraints on identifying roles (graph schemas where the entity plays role 0 with a UC on role 1's value type noun). Populated during `nouns()` by cross-referencing constraint spans after loading constraints.
- `NounDef.permissions` is `string[]` — NOT a DB column. This is a runtime field from Payload collection access config, not stored in SQLite. DomainModel should accept an optional `permissions` map in the constructor or set a default `['list', 'read', 'create', 'update', 'delete']` for all entity nouns.
- `NounDef.description` maps from `prompt_text` column in DB
- `NounDef.exclusiveMinimum`, `exclusiveMaximum`, `minLength`, `maxLength`, `multipleOf` — NOT in current DB DDL. Include in the type for spec completeness; they will be `undefined` until DDL migrations are added. Only `minimum`, `maximum`, `pattern` exist in the nouns table today.
- `ConstraintDef.text` — added via ALTER TABLE migration in do.ts, exists at runtime
- `ConstraintDef.setComparisonArgumentLength` — added via ALTER TABLE migration, exists at runtime
- `ConstraintDef.deonticOperator` — NOT a DB column. Derived from `text` pattern: if text starts with "It is obligatory that" → `'obligatory'`, "It is forbidden that" → `'forbidden'`, "It is permitted that" → `'permitted'`. DomainModel parses this during `constraints()`.
- `ConstraintDef.entity` and `ConstraintDef.clauses` — NOT DB columns. Derived during constraint processing (entity from constraint spans, clauses from text parsing). May remain `undefined` if not derivable.
- `SpanDef.subsetAutofill` — added via ALTER TABLE migration (INTEGER DEFAULT 0), exists at runtime
- `StateMachineDef` includes `nounDef: NounDef` (resolved)
- `TransitionDef` includes `eventTypeId: string` and optional `VerbDef`
- `VerbDef.func` — the `functions` table has: `callback_url`, `http_method`, `headers`, `verb_id`. There is NO `function_type` column in the DB. Remove `functionType` from `VerbDef.func` — or include it as optional and always `undefined` until a migration is added.
- `VerbDef.func.headers` is stored as TEXT (JSON string) in DB — DomainModel parses to `Record<string, string>`

- [ ] **Step 4: Run test — expect PASS**

Run: `npx vitest run src/model/types.test.ts`
Expected: PASS (all type checks compile, runtime assertions pass)

- [ ] **Step 5: Commit**

```bash
git add src/model/types.ts src/model/types.test.ts
git commit -m "feat(model): add shared DomainModel type definitions"
```

---

### Task 2: Renderer Interfaces + render() Function

**Files:**
- Create: `src/model/renderer.ts`
- Test: `src/model/renderer.test.ts`

- [ ] **Step 1: Write test for render() walker**

```typescript
// src/model/renderer.test.ts
import { describe, it, expect } from 'vitest'
import { render } from './renderer'
import type { NounDef, FactTypeDef, ConstraintDef, StateMachineDef, Generator } from './types'

// Minimal mock model implementing the accessor interface render() needs
function mockAccessors(data: {
  nouns?: NounDef[],
  factTypes?: FactTypeDef[],
  constraints?: ConstraintDef[],
  stateMachines?: StateMachineDef[],
}) {
  const nounMap = new Map((data.nouns ?? []).map(n => [n.name, n]))
  const ftMap = new Map((data.factTypes ?? []).map(ft => [ft.id, ft]))
  return {
    nouns: async () => nounMap,
    factTypes: async () => ftMap,
    factTypesFor: async (noun: NounDef) => (data.factTypes ?? []).filter(ft =>
      ft.roles.some(r => r.nounDef.name === noun.name && r.roleIndex === 0)),
    constraintsFor: async (fts: FactTypeDef[]) => {
      const ids = new Set(fts.map(f => f.id))
      return (data.constraints ?? []).filter(c => c.spans.some(s => ids.has(s.factTypeId)))
    },
    constraints: async () => data.constraints ?? [],
    stateMachines: async () => new Map((data.stateMachines ?? []).map(sm => [sm.id, sm])),
  }
}

describe('render()', () => {
  const customer: NounDef = { id: 'n1', name: 'Customer', objectType: 'entity', domainId: 'd1' }
  const email: NounDef = { id: 'n2', name: 'Email', objectType: 'value', domainId: 'd1', valueType: 'string' }

  it('visits entity nouns with their fact types and constraints', async () => {
    const ft: FactTypeDef = {
      id: 'gs1', reading: 'Customer has Email', arity: 2,
      roles: [
        { id: 'r1', nounName: 'Customer', nounDef: customer, roleIndex: 0 },
        { id: 'r2', nounName: 'Email', nounDef: email, roleIndex: 1 },
      ],
    }
    const uc: ConstraintDef = {
      id: 'c1', kind: 'UC', modality: 'Alethic',
      text: 'Each Customer has at most one Email',
      spans: [{ factTypeId: 'gs1', roleIndex: 1 }],
    }

    const gen: Generator<string, string[]> = {
      noun: {
        entity: (noun, fts, cs) => `entity:${noun.name}(fts=${fts.length},cs=${cs.length})`,
        value: (noun) => `value:${noun.name}`,
      },
      combine: (parts) => parts,
    }

    const result = await render(mockAccessors({ nouns: [customer, email], factTypes: [ft], constraints: [uc] }), gen)
    expect(result).toContain('entity:Customer(fts=1,cs=1)')
    expect(result).toContain('value:Email')
  })

  it('visits fact types by arity when factType renderer provided', async () => {
    const ft: FactTypeDef = {
      id: 'gs1', reading: 'Customer has Email', arity: 2,
      roles: [
        { id: 'r1', nounName: 'Customer', nounDef: customer, roleIndex: 0 },
        { id: 'r2', nounName: 'Email', nounDef: email, roleIndex: 1 },
      ],
    }

    const gen: Generator<string, string[]> = {
      noun: { entity: () => 'e', value: () => 'v' },
      factType: {
        binary: (entityRole, valueRole, cs) => `binary:${entityRole.nounName}->${valueRole.nounName}`,
      },
      combine: (parts) => parts,
    }

    const result = await render(mockAccessors({ nouns: [customer, email], factTypes: [ft] }), gen)
    expect(result).toContain('binary:Customer->Email')
  })

  it('visits state machines when stateMachine renderer provided', async () => {
    const sm: StateMachineDef = {
      id: 'sm1', nounName: 'Order', nounDef: customer, // reusing customer as placeholder
      statuses: [{ id: 's1', name: 'Draft' }],
      transitions: [{ from: 'Draft', to: 'Pending', event: 'Submit', eventTypeId: 'et1' }],
    }

    const gen: Generator<string, string[]> = {
      noun: { entity: () => 'e', value: () => 'v' },
      stateMachine: (sm) => `sm:${sm.nounName}(${sm.statuses.length} states)`,
      combine: (parts) => parts,
    }

    const result = await render(mockAccessors({ nouns: [customer], stateMachines: [sm] }), gen)
    expect(result).toContain('sm:Order(1 states)')
  })
})
```

- [ ] **Step 2: Run test — expect FAIL (module not found)**

Run: `npx vitest run src/model/renderer.test.ts`
Expected: FAIL

- [ ] **Step 3: Create renderer.ts**

Create `src/model/renderer.ts` with:
1. `NounRenderer<T>`, `FactTypeRenderer<T>`, `Generator<T, Out>` interfaces (from spec §2)
2. `ModelAccessors` interface (the subset of DomainModel methods that `render()` needs)
3. `render<T, Out>(model: ModelAccessors, gen: Generator<T, Out>): Promise<Out>` function — the walker implementation from spec §2

The `render()` function:
- Iterates nouns: entity nouns get `gen.noun.entity(noun, fts, cs)`, value nouns get `gen.noun.value(noun)`
- If `gen.factType` provided: iterates all fact types, dispatches to `unary/binary/nary/custom` by arity
- If `gen.constraint` provided: iterates all constraints
- If `gen.stateMachine` provided: iterates all state machines
- Returns `gen.combine(parts)`

Add note in JSDoc: "Generators should use EITHER per-entity fact types in noun.entity() OR the global factType renderer for a given concern — not both, to avoid double-processing."

Also re-export the `Generator`, `NounRenderer`, `FactTypeRenderer` types from this file for convenience.

- [ ] **Step 4: Run test — expect PASS**

Run: `npx vitest run src/model/renderer.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/model/renderer.ts src/model/renderer.test.ts
git commit -m "feat(model): add renderer interfaces and render() walker"
```

---

### Task 3: DomainModel Class

**Files:**
- Create: `src/model/domain-model.ts`
- Test: `src/model/domain-model.test.ts`

The DomainModel wraps `SqlStorage` and provides typed, cached accessors. Each accessor runs SQL against the DO's SQLite, resolves FKs, and caches the result.

**Important context for implementer:**
- `SqlStorage.exec(query, ...bindings)` returns an iterable cursor of row objects (column names as keys)
- All tables use snake_case columns (see `src/schema/metamodel.ts` and `src/schema/state.ts`)
- DomainModel is scoped to a single `domainId`
- Some collections (roles, constraint-spans) are NOT domain-scoped in the DB — filter by joining through domain-scoped tables
- `enum_values` in nouns table is a JSON string — parse to `string[]`
- `prompt_text` column maps to `description` in NounDef

- [ ] **Step 1: Write test with mock SqlStorage**

Create a test helper that builds a mock `SqlStorage` from fixture data. The mock intercepts `exec()` calls, parses the SQL query's `FROM` table name and `WHERE` conditions, and returns matching rows from in-memory arrays.

```typescript
// src/model/domain-model.test.ts
import { describe, it, expect, vi } from 'vitest'
import { DomainModel } from './domain-model'

// Helper: build a mock SqlStorage that serves rows from in-memory data
function mockSqlStorage(tables: Record<string, Record<string, any>[]>) {
  return {
    exec: (query: string, ...bindings: any[]) => {
      // Return an iterable of row objects
      // The DomainModel implementation will use specific known queries
      // This mock stores queries and returns pre-configured results
      const rows = tables[query] ?? []
      return { [Symbol.iterator]: () => rows[Symbol.iterator](), ...rows }
    },
  }
}
```

Actually, since DomainModel will use specific SQL strings, mock SqlStorage by matching the query string against known patterns. A simpler approach: the DomainModel can use a data-loading abstraction that's easy to mock. But the spec says it wraps SqlStorage directly.

**Alternative approach for testability**: Create a `DataLoader` interface that DomainModel's constructor accepts. In production, the loader runs SQL. In tests, the loader returns mock data. This is cleaner than parsing SQL in mocks.

Row types use `Record<string, any>` since `SqlStorage.exec()` returns untyped row objects with column names as keys. The DataLoader methods return arrays of these untyped rows — DomainModel handles the mapping to typed defs.

```typescript
// In domain-model.ts:

// All row types are untyped DB row objects (column names as keys)
type Row = Record<string, any>

export interface DataLoader {
  queryNouns(domainId: string): Row[]
  queryGraphSchemas(domainId: string): Row[]
  queryReadings(domainId: string): Row[]
  queryRoles(): Row[]
  queryConstraints(domainId: string): Row[]
  queryConstraintSpans(): Row[]
  queryStateMachineDefs(domainId: string): Row[]
  queryStatuses(domainId: string): Row[]
  queryTransitions(domainId: string): Row[]
  queryEventTypes(domainId: string): Row[]
  queryGuards(domainId: string): Row[]
  queryVerbs(domainId: string): Row[]
  queryFunctions(domainId: string): Row[]
}

// Production implementation:
export class SqlDataLoader implements DataLoader {
  constructor(private sql: SqlStorage) {}
  queryNouns(domainId: string) {
    return [...this.sql.exec('SELECT n.*, p.name as super_type_name FROM nouns n LEFT JOIN nouns p ON n.super_type_id = p.id WHERE n.domain_id = ?', domainId)]
  }
  // ... etc (one method per table, each returns Row[])
}
```

Test with mock DataLoader:

```typescript
describe('DomainModel', () => {
  const DOMAIN = 'd1'

  function mockLoader(data: Partial<Record<string, any[]>>): DataLoader {
    return {
      queryNouns: () => data.nouns ?? [],
      queryGraphSchemas: () => data.graphSchemas ?? [],
      queryReadings: () => data.readings ?? [],
      queryRoles: () => data.roles ?? [],
      queryConstraints: () => data.constraints ?? [],
      queryConstraintSpans: () => data.constraintSpans ?? [],
      queryStateMachineDefs: () => data.smDefs ?? [],
      queryStatuses: () => data.statuses ?? [],
      queryTransitions: () => data.transitions ?? [],
      queryEventTypes: () => data.eventTypes ?? [],
      queryGuards: () => data.guards ?? [],
      queryVerbs: () => data.verbs ?? [],
      queryFunctions: () => data.functions ?? [],
    } as DataLoader
  }

  describe('nouns()', () => {
    it('returns entity and value nouns with resolved superTypes', async () => {
      const model = new DomainModel(mockLoader({
        nouns: [
          { id: 'n1', name: 'Person', object_type: 'entity', domain_id: DOMAIN, super_type_id: null },
          { id: 'n2', name: 'Customer', object_type: 'entity', domain_id: DOMAIN, super_type_id: 'n1', super_type_name: 'Person' },
          { id: 'n3', name: 'Name', object_type: 'value', domain_id: DOMAIN, value_type: 'string' },
        ],
      }), DOMAIN)

      const nouns = await model.nouns()
      expect(nouns.size).toBe(3)
      expect(nouns.get('Customer')?.superType).toBe('Person')
      expect(nouns.get('Name')?.valueType).toBe('string')
    })

    it('parses enum_values from JSON string', async () => {
      const model = new DomainModel(mockLoader({
        nouns: [{ id: 'n1', name: 'Priority', object_type: 'value', domain_id: DOMAIN, enum_values: '["low","medium","high"]' }],
      }), DOMAIN)

      const nouns = await model.nouns()
      expect(nouns.get('Priority')?.enumValues).toEqual(['low', 'medium', 'high'])
    })

    it('caches results across calls', async () => {
      const loader = mockLoader({ nouns: [{ id: 'n1', name: 'X', object_type: 'entity', domain_id: DOMAIN }] })
      const spy = vi.spyOn(loader, 'queryNouns')
      const model = new DomainModel(loader, DOMAIN)

      await model.nouns()
      await model.nouns()
      expect(spy).toHaveBeenCalledTimes(1)
    })
  })

  describe('factTypes()', () => {
    it('groups roles by graph schema and resolves noun references', async () => {
      const model = new DomainModel(mockLoader({
        nouns: [
          { id: 'n1', name: 'Customer', object_type: 'entity', domain_id: DOMAIN },
          { id: 'n2', name: 'Name', object_type: 'value', domain_id: DOMAIN, value_type: 'string' },
        ],
        graphSchemas: [{ id: 'gs1', name: 'CustomerHasName', domain_id: DOMAIN }],
        readings: [{ id: 'rd1', text: 'Customer has Name', graph_schema_id: 'gs1', domain_id: DOMAIN }],
        roles: [
          { id: 'r1', noun_id: 'n1', graph_schema_id: 'gs1', role_index: 0 },
          { id: 'r2', noun_id: 'n2', graph_schema_id: 'gs1', role_index: 1 },
        ],
      }), DOMAIN)

      const fts = await model.factTypes()
      expect(fts.size).toBe(1)
      const ft = fts.get('gs1')!
      expect(ft.reading).toBe('Customer has Name')
      expect(ft.arity).toBe(2)
      expect(ft.roles[0].nounName).toBe('Customer')
      expect(ft.roles[1].nounDef.valueType).toBe('string')
    })
  })

  describe('constraints()', () => {
    it('groups spans by constraint and resolves role indices', async () => {
      const model = new DomainModel(mockLoader({
        nouns: [{ id: 'n1', name: 'Customer', object_type: 'entity', domain_id: DOMAIN }],
        roles: [{ id: 'r1', noun_id: 'n1', graph_schema_id: 'gs1', role_index: 0 }],
        constraints: [{ id: 'c1', kind: 'UC', modality: 'Alethic', domain_id: DOMAIN }],
        constraintSpans: [{ id: 'cs1', constraint_id: 'c1', role_id: 'r1' }],
      }), DOMAIN)

      const cs = await model.constraints()
      expect(cs).toHaveLength(1)
      expect(cs[0].kind).toBe('UC')
      expect(cs[0].spans).toHaveLength(1)
      expect(cs[0].spans[0].factTypeId).toBe('gs1')
    })
  })

  describe('stateMachines()', () => {
    it('resolves full transition chain: status → event → verb → function', async () => {
      const model = new DomainModel(mockLoader({
        nouns: [{ id: 'n1', name: 'Order', object_type: 'entity', domain_id: DOMAIN }],
        smDefs: [{ id: 'smd1', noun_id: 'n1', domain_id: DOMAIN }],
        statuses: [
          { id: 's1', name: 'Draft', state_machine_definition_id: 'smd1', created_at: '2026-01-01' },
          { id: 's2', name: 'Pending', state_machine_definition_id: 'smd1', created_at: '2026-01-02' },
        ],
        transitions: [{ id: 't1', from_status_id: 's1', to_status_id: 's2', event_type_id: 'et1', verb_id: 'v1', domain_id: DOMAIN }],
        eventTypes: [{ id: 'et1', name: 'Submit', domain_id: DOMAIN }],
        verbs: [{ id: 'v1', name: 'submitOrder', domain_id: DOMAIN }],
        functions: [{ id: 'f1', verb_id: 'v1', callback_url: '/api/submit', http_method: 'POST', headers: null, domain_id: DOMAIN }],
      }), DOMAIN)

      const sms = await model.stateMachines()
      expect(sms.size).toBe(1)
      const sm = sms.get('smd1')!
      expect(sm.nounName).toBe('Order')
      expect(sm.statuses).toHaveLength(2)
      expect(sm.transitions[0].from).toBe('Draft')
      expect(sm.transitions[0].to).toBe('Pending')
      expect(sm.transitions[0].event).toBe('Submit')
      expect(sm.transitions[0].verb?.func?.callbackUrl).toBe('/api/submit')
    })
  })

  describe('invalidate()', () => {
    it('clears specific cache keys by collection', async () => {
      const loader = mockLoader({ nouns: [{ id: 'n1', name: 'X', object_type: 'entity', domain_id: DOMAIN }] })
      const model = new DomainModel(loader, DOMAIN)

      await model.nouns() // populate cache
      model.invalidate('nouns')

      const spy = vi.spyOn(loader, 'queryNouns')
      await model.nouns() // should re-query
      expect(spy).toHaveBeenCalledTimes(1)
    })

    it('clears all caches when no collection specified', async () => {
      const loader = mockLoader({
        nouns: [{ id: 'n1', name: 'X', object_type: 'entity', domain_id: DOMAIN }],
        constraints: [{ id: 'c1', kind: 'UC', modality: 'Alethic', domain_id: DOMAIN }],
      })
      const model = new DomainModel(loader, DOMAIN)

      await model.nouns()
      await model.constraints()
      model.invalidate()

      const nounSpy = vi.spyOn(loader, 'queryNouns')
      const cSpy = vi.spyOn(loader, 'queryConstraints')
      await model.nouns()
      await model.constraints()
      expect(nounSpy).toHaveBeenCalledTimes(1)
      expect(cSpy).toHaveBeenCalledTimes(1)
    })
  })
})
```

- [ ] **Step 2: Run test — expect FAIL**

Run: `npx vitest run src/model/domain-model.test.ts`
Expected: FAIL — module not found

- [ ] **Step 3: Implement DomainModel class**

Create `src/model/domain-model.ts`:

1. `DataLoader` interface with query methods for each collection (returns raw DB rows)
2. `SqlDataLoader` class implementing `DataLoader` using `SqlStorage.exec()` — each method runs a specific SQL query against the DO's SQLite
3. `DomainModel` class:
   - Constructor: `(loader: DataLoader, domainId: string)`
   - Private `cache: Map<string, any>`
   - `INVALIDATION_MAP` constant (from spec §4)
   - `nouns()` — query nouns, resolve superType names, parse enumValues JSON, build NounDef map
   - `noun(name)` — delegate to nouns()
   - `factTypes()` — query graph_schemas + readings + roles, group roles by graph_schema_id, resolve noun references via nouns(), build FactTypeDef map
   - `factTypesFor(noun)` — filter factTypes where noun plays role at index 0 (subject position)
   - `constraints()` — query constraints + constraint_spans + roles, group spans by constraint_id, resolve factTypeId from role's graph_schema_id
   - `constraintsFor(fts)` — filter constraints where any span's factTypeId matches
   - `constraintSpans()` — return Map<constraintId, SpanDef[]>
   - `stateMachines()` — query state_machine_definitions + statuses + transitions + event_types + verbs + functions + guards. Build full resolved StateMachineDef with StatusDef[], TransitionDef[] (with VerbDef and guard)
   - `readings()` — query readings + roles, resolve role noun references, build ReadingDef[]
   - `render(gen)` — delegate to the standalone `render()` from renderer.ts, passing `this` as the model
   - `invalidate(collection?)` — use INVALIDATION_MAP to clear affected cache keys

SQL queries (key ones for the implementer):

**nouns:**
```sql
SELECT n.*, p.name as super_type_name
FROM nouns n
LEFT JOIN nouns p ON n.super_type_id = p.id
WHERE n.domain_id = ?
```

**factTypes (graph_schemas + readings + roles):**
```sql
SELECT gs.id as gs_id, gs.name as gs_name, gs.title,
       r.id as reading_id, r.text as reading_text,
       role.id as role_id, role.noun_id, role.role_index
FROM graph_schemas gs
LEFT JOIN readings r ON r.graph_schema_id = gs.id
LEFT JOIN roles role ON role.graph_schema_id = gs.id
WHERE gs.domain_id = ?
ORDER BY gs.id, role.role_index
```
Then group by gs_id, resolve noun references from the nouns cache.

**constraints + spans:**
```sql
SELECT c.*, cs.id as span_id, cs.role_id, cs.subset_autofill,
       role.graph_schema_id as fact_type_id, role.role_index
FROM constraints c
LEFT JOIN constraint_spans cs ON cs.constraint_id = c.id
LEFT JOIN roles role ON role.id = cs.role_id
WHERE c.domain_id = ?
ORDER BY c.id
```
Then group by c.id to build SpanDef arrays.

**stateMachines (big join):**
```sql
SELECT smd.id as smd_id, smd.noun_id,
       s.id as status_id, s.name as status_name, s.created_at as status_created,
       t.id as transition_id, t.from_status_id, t.to_status_id, t.event_type_id, t.verb_id,
       et.name as event_name,
       v.id as v_id, v.name as verb_name, v.status_id as v_status_id,
       v.transition_id as v_transition_id, v.graph_id as v_graph_id,
       v.agent_definition_id as v_agent_def_id,
       f.callback_url, f.http_method, f.headers,
       g.id as guard_id, g.graph_schema_id as guard_gs_id
FROM state_machine_definitions smd
LEFT JOIN statuses s ON s.state_machine_definition_id = smd.id
LEFT JOIN transitions t ON t.from_status_id = s.id
LEFT JOIN event_types et ON et.id = t.event_type_id
LEFT JOIN verbs v ON v.id = t.verb_id
LEFT JOIN functions f ON f.verb_id = v.id
LEFT JOIN guards g ON g.transition_id = t.id
WHERE smd.domain_id = ?
ORDER BY smd.id, s.created_at, t.id
```
Then group by smd_id → status → transition. **Important**: The join produces duplicate status rows when a status has multiple transitions. Use `Map<string, StatusDef>` keyed by status_id and `Map<string, TransitionDef>` keyed by transition_id to deduplicate. Also parse `f.headers` from JSON string to `Record<string, string>` (or `undefined` if null).

**deonticOperator derivation in constraints()**: After loading constraints, if `modality === 'Deontic'` and `text` is present, derive `deonticOperator` from text pattern:
- text starts with "It is obligatory that" → `'obligatory'`
- text starts with "It is forbidden that" → `'forbidden'`
- text starts with "It is permitted that" → `'permitted'`

- [ ] **Step 4: Run test — expect PASS**

Run: `npx vitest run src/model/domain-model.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/model/domain-model.ts src/model/domain-model.test.ts
git commit -m "feat(model): add DomainModel with lazy cached accessors"
```

---

### Task 4: Test Utilities — Mock DomainModel Builder

**Files:**
- Create: `src/model/test-utils.ts`
- Test: `src/model/test-utils.test.ts`

Generators need a mock DomainModel for testing. This utility builds one from typed data (NounDef[], FactTypeDef[], etc.) without requiring SqlStorage.

- [ ] **Step 1: Write test**

```typescript
// src/model/test-utils.test.ts
import { describe, it, expect } from 'vitest'
import { createMockModel, mkNounDef, mkValueNounDef, mkFactType, mkConstraint } from './test-utils'

describe('createMockModel', () => {
  it('returns a DomainModel-compatible object', async () => {
    const customer = mkNounDef({ name: 'Customer' })
    const name = mkValueNounDef({ name: 'Name', valueType: 'string' })
    const ft = mkFactType({
      reading: 'Customer has Name',
      roles: [
        { nounDef: customer, roleIndex: 0 },
        { nounDef: name, roleIndex: 1 },
      ],
    })
    const uc = mkConstraint({ kind: 'UC', spans: [{ factTypeId: ft.id, roleIndex: 1 }] })

    const model = createMockModel({ nouns: [customer, name], factTypes: [ft], constraints: [uc] })

    const nouns = await model.nouns()
    expect(nouns.size).toBe(2)
    expect(nouns.get('Customer')?.objectType).toBe('entity')

    const fts = await model.factTypesFor(customer)
    expect(fts).toHaveLength(1)

    const cs = await model.constraintsFor([ft])
    expect(cs).toHaveLength(1)
    expect(cs[0].kind).toBe('UC')
  })
})
```

- [ ] **Step 2: Run test — expect FAIL**

- [ ] **Step 3: Implement test utilities**

Create `src/model/test-utils.ts`:

1. **Factory functions** (auto-generate IDs using a counter):
   - `mkNounDef(overrides)` → NounDef with defaults: `{ objectType: 'entity', domainId: 'd1' }`
   - `mkValueNounDef(overrides)` → NounDef with defaults: `{ objectType: 'value', domainId: 'd1' }`
   - `mkFactType({ reading, roles })` → FactTypeDef with auto-generated id, arity from roles.length. `roles` accepts shorthand `{ nounDef, roleIndex }` and auto-fills id/nounName.
   - `mkConstraint({ kind, spans, ... })` → ConstraintDef with auto-generated id, default modality 'Alethic'
   - `mkStateMachine({ nounDef, statuses, transitions })` → StateMachineDef

2. **`createMockModel(data)`** — builds a DomainModel-compatible object:
   - `nouns()` → Map from data.nouns
   - `noun(name)` → lookup
   - `factTypes()` → Map from data.factTypes
   - `factTypesFor(noun)` → filter where noun is at roleIndex 0
   - `constraints()` → data.constraints
   - `constraintsFor(fts)` → filter where span.factTypeId matches any ft.id
   - `constraintSpans()` → build Map from constraints' spans
   - `stateMachines()` → Map from data.stateMachines
   - `readings()` → data.readings
   - `render(gen)` → delegate to `render()` from renderer.ts
   - `invalidate()` → no-op

- [ ] **Step 4: Run test — expect PASS**

- [ ] **Step 5: Commit**

```bash
git add src/model/test-utils.ts src/model/test-utils.test.ts
git commit -m "feat(model): add test utilities for mock DomainModel"
```

---

### Task 5: DO Integration — getModel(), Auto-Invalidation, generate()

**Files:**
- Modify: `src/do.ts`
- Modify: `src/api/generate.ts`
- Test: existing `src/do.test.ts` (if it exists, verify), `src/api/generate.test.ts`

- [ ] **Step 1: Add DomainModel imports and getModel() to DO**

In `src/do.ts`, add:
```typescript
import { DomainModel, SqlDataLoader } from './model/domain-model'

// After class declaration, add instance variable:
private models: Map<string, DomainModel> = new Map()

getModel(domainId: string): DomainModel {
  let model = this.models.get(domainId)
  if (!model) {
    model = new DomainModel(new SqlDataLoader(this.sql), domainId)
    this.models.set(domainId, model)
  }
  return model
}
```

- [ ] **Step 2: Add auto-invalidation to write methods**

In `createInCollection()`, after the `_insertRow` call and CDC log, add auto-invalidation. The data has already been translated via `payloadToRow()` into `sqlData` at this point:
```typescript
// Auto-invalidate DomainModel cache
const domainId = sqlData.domain_id as string
if (domainId) this.getModel(domainId).invalidate(collectionSlug)
```

Same pattern in `updateInCollection()` — `sqlData` is available after `payloadToRow()`. Extract `domainId` from `sqlData.domain_id`.

Same in `deleteFromCollection()` — the record is fetched first (to get the current row for CDC). Extract `domainId` from the fetched row's `domain_id` column, then invalidate after delete.

- [ ] **Step 3: Add generate() RPC method to DO**

```typescript
async generate(domainId: string, format: string): Promise<any> {
  const model = this.getModel(domainId)
  switch (format) {
    case 'openapi':
      return generateOpenAPI(model)
    case 'sqlite':
      return generateSQLite(await generateOpenAPI(model))
    case 'xstate':
      return generateXState(model)
    case 'ilayer':
      return model.render(ilayerGenerator)
    case 'readings':
      return model.render(readingsGenerator)
    case 'constraint-ir':
      return model.render(constraintIrGenerator)
    case 'mdxui':
      return model.render(mdxuiGenerator)
    default:
      throw new Error(`Unknown format: ${format}`)
  }
}
```

**Important:** This method is added now but references generator functions that are rewritten in later tasks. Use the OLD generator signatures initially — they will be updated as each generator is rewritten in Chunk 2. The transitional version:

```typescript
// Transitional — delegates to old generator signatures
// Each generator import will be updated in Chunk 2
async generate(domainId: string, format: string): Promise<any> {
  switch (format) {
    case 'openapi':
      return (await import('./generate/openapi')).generateOpenAPI(this, domainId)
    case 'sqlite':
      return (await import('./generate/sqlite')).generateSQLite(
        await (await import('./generate/openapi')).generateOpenAPI(this, domainId))
    case 'xstate':
      return (await import('./generate/xstate')).generateXState(this, domainId)
    case 'ilayer':
      return (await import('./generate/ilayer')).generateILayer(this, domainId)
    case 'readings':
      return (await import('./generate/readings')).generateReadings(this, domainId)
    case 'constraint-ir':
      return (await import('./generate/constraint-ir')).generateConstraintIR(this, domainId)
    default:
      throw new Error(`Unknown format: ${format}`)
  }
}
```

**RPC mechanism:** Cloudflare Durable Objects expose public methods on the DO class as RPC automatically. The Worker gets a `DurableObjectStub` via `env.GRAPHDL_DB.get(id)`, and calls `stub.generate(domainId, format)` — Cloudflare serializes args and returns the result. No custom fetch handler needed.

- [ ] **Step 4: Update generate.ts handler**

In `src/api/generate.ts`, change the generator dispatch to call the DO's `generate()` RPC:
```typescript
// Before: Worker imports and calls each generator directly
// After: Worker calls DO's generate() method via RPC
const output = await db.generate(domainId, format)
```

Remove all the individual generator imports and the switch statement from `generate.ts`. The DO's `generate()` method handles dispatch.

Do NOT add `'mdxui'` to the valid formats yet — it will be added in Chunk 3 Task 15 when the generator exists.

- [ ] **Step 5: Run existing tests**

Run: `npx vitest run src/api/generate.test.ts`
Expected: PASS (handler tests should still work since the DO stub can proxy the generate() call)

- [ ] **Step 6: Commit**

```bash
git add src/do.ts src/api/generate.ts
git commit -m "feat(do): add getModel(), auto-invalidation, and generate() RPC"
```

---

### Task 6: Barrel Export

**Files:**
- Create: `src/model/index.ts`

- [ ] **Step 1: Create barrel export**

```typescript
// src/model/index.ts
export type { NounDef, FactTypeDef, RoleDef, ConstraintDef, SpanDef, StateMachineDef, StatusDef, TransitionDef, VerbDef, ReadingDef } from './types'
export type { Generator, NounRenderer, FactTypeRenderer } from './renderer'
export { render } from './renderer'
export { DomainModel, SqlDataLoader } from './domain-model'
export type { DataLoader } from './domain-model'
```

- [ ] **Step 2: Commit**

```bash
git add src/model/index.ts
git commit -m "feat(model): add barrel export"
```

---

## Chunk 2: Generator Rewrites

### Task 7: Refactor rmap.ts — NounRef → NounDef

**Files:**
- Modify: `src/generate/rmap.ts`
- Test: `src/generate/rmap.test.ts` (existing, should still pass)

- [ ] **Step 1: Replace NounRef with NounDef import**

In `src/generate/rmap.ts`:
1. Remove the `NounRef` interface definition (lines 11-30)
2. Add `import type { NounDef } from '../model/types'`
3. Replace all `NounRef` references with `NounDef`
4. The function signatures and logic remain the same — `NounDef` has all the same fields that `NounRef` had (plus more)

The key type differences:
- `NounRef.enumValues` was `string | null` → `NounDef.enumValues` is `string[] | undefined` (pre-parsed)
- `NounRef.superType` was `string | NounRef | null` → `NounDef.superType` is `NounDef | string | undefined`
- `NounRef.referenceScheme` was `(string | NounRef)[] | null` → `NounDef.referenceScheme` is `NounDef[] | undefined`

Check if any rmap.ts functions depend on the old types (e.g., null checks vs undefined checks). Adjust if needed but rmap.ts functions mostly just read `.name` and `.id` which are the same.

- [ ] **Step 2: Run existing tests**

Run: `npx vitest run src/generate/rmap.test.ts`
Expected: PASS — rmap functions are pure string manipulation, type change is transparent

- [ ] **Step 3: Commit**

```bash
git add src/generate/rmap.ts
git commit -m "refactor(generate): replace NounRef with NounDef in rmap.ts"
```

---

### Task 8: Refactor schema-builder.ts — Accept NounDef

**Files:**
- Modify: `src/generate/schema-builder.ts`
- Test: `src/generate/schema-builder.test.ts` (existing)

- [ ] **Step 1: Update imports and function signatures**

1. Replace `import { NounRef } from './rmap'` with `import type { NounDef } from '../model/types'`
2. Replace all `NounRef` type annotations with `NounDef`
3. Key contract changes:
   - `enumValues.split(',')` → iterate `enumValues` directly (it's already `string[]`)
   - `referenceScheme` elements are now `NounDef` objects, not `string | NounRef` — update recursive `createProperty` calls to accept `NounDef` directly
   - `superType` traversal: was `typeof superType === 'string' ? lookup(superType) : superType` → now always `NounDef | string`, same pattern but simpler

- [ ] **Step 2: Fix enumValues handling**

Find the `enumValues.split(',')` call and change to:
```typescript
// Before:
if (object.enumValues) {
  const vals = object.enumValues.split(',').map(v => v.trim())
  // ...
}
// After:
if (object.enumValues) {
  const vals = object.enumValues  // already string[]
  // ...
}
```

- [ ] **Step 3: Fix referenceScheme handling**

The `createProperty` function recursively processes referenceScheme elements. Update to handle `NounDef[]`:
```typescript
// Before: referenceScheme elements could be string IDs or NounRef objects
// After: referenceScheme elements are always NounDef objects
if (subject.referenceScheme) {
  for (const refNoun of subject.referenceScheme) {
    // refNoun is now always NounDef, no string ID case needed
    createProperty({ tables, object: refNoun, ... })
  }
}
```

- [ ] **Step 4: Run existing tests**

Run: `npx vitest run src/generate/schema-builder.test.ts`

The test file creates NounRef-shaped objects. If tests use `enumValues: 'a,b,c'` (string), update test fixtures to use `enumValues: ['a', 'b', 'c']` (string array). Similarly for `referenceScheme` — update from `(string | NounRef)[]` to `NounDef[]`.

Expected: PASS after fixture updates

- [ ] **Step 5: Commit**

```bash
git add src/generate/schema-builder.ts src/generate/schema-builder.test.ts
git commit -m "refactor(generate): update schema-builder for NounDef types"
```

---

### Task 9: Refactor fact-processors.ts — Accept DomainModel Types

**Files:**
- Modify: `src/generate/fact-processors.ts`
- Test: `src/generate/fact-processors.test.ts` (existing)

- [ ] **Step 1: Update imports**

Replace `NounRef` imports with `NounDef` from model types. The three processor functions (`processBinarySchemas`, `processArraySchemas`, `processUnarySchemas`) accept collections of raw DB rows. Update their parameter types to accept DomainModel types or compatible shapes.

The processors are called by the OpenAPI generator. Their internal logic does:
- Tokenize readings by noun names (via rmap.ts)
- Find property names from readings
- Call schema-builder functions (now updated for NounDef)

The parameter shapes used by fact-processors need to match what the OpenAPI generator will pass. Since OpenAPI becomes a direct generator using DomainModel, the processors will receive FactTypeDef[], ConstraintDef[], etc.

- [ ] **Step 2: Update processor function signatures**

Update to accept DomainModel types. The OpenAPI generator (Task 11) will pass these types. Define the new signatures now so both tasks agree on the contract:

```typescript
// fact-processors.ts — new signatures

// processBinarySchemas: handles binary fact types with single-role UCs
export function processBinarySchemas(
  tables: Tables,                        // accumulator (same as before)
  factTypes: FactTypeDef[],              // was: readings/roles/graphSchemas raw rows
  constraints: ConstraintDef[],          // was: raw constraint rows
  constraintSpans: Map<string, SpanDef[]>, // was: raw constraint_span rows
  nouns: Map<string, NounDef>,           // was: noun lookup object
): void

// processArraySchemas: handles compound UCs (multi-role spanning constraints)
export function processArraySchemas(
  tables: Tables,
  factTypes: FactTypeDef[],
  constraints: ConstraintDef[],
  constraintSpans: Map<string, SpanDef[]>,
  nouns: Map<string, NounDef>,
): void

// processUnarySchemas: handles unary fact types → boolean properties
export function processUnarySchemas(
  tables: Tables,
  factTypes: FactTypeDef[],              // filter for arity === 1
): void
```

The key mapping from old to new:
- Old: iterate raw readings → find roles → look up nouns by ID → call rmap functions
- New: iterate `FactTypeDef[]` → roles and noun references are pre-resolved in `FactTypeDef.roles[].nounDef`
- Old: `constraintSpans` was array of raw rows with `constraint_id`, `role_id` → now `Map<constraintId, SpanDef[]>` with `factTypeId` and `roleIndex`
- Old: noun lookup by ID → now `Map<name, NounDef>` (name-keyed)

- [ ] **Step 3: Run existing tests and fix fixtures**

Run: `npx vitest run src/generate/fact-processors.test.ts`

Update test fixtures for the NounDef type changes (same as schema-builder).

Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/generate/fact-processors.ts src/generate/fact-processors.test.ts
git commit -m "refactor(generate): update fact-processors for NounDef types"
```

---

### Task 10: Rewrite constraint-ir Generator (Direct)

**Files:**
- Modify: `src/generate/constraint-ir.ts`
- Modify: `src/generate/constraint-ir.test.ts`

The constraint-ir generator is the simplest rewrite — the DomainModel already IS the constraint IR. The generator serializes the typed data to the output shape expected by the WASM evaluator. This is a **direct generator** (not a walker) because the IR output is a coordinated whole — nouns, factTypes, constraints, and stateMachines all appear in one flat JSON blob, not decomposed per-entity.

- [ ] **Step 1: Rewrite generator as direct DomainModel consumer**

Replace `generateConstraintIR(db, domainId)` with `generateConstraintIR(model: DomainModel)`:

```typescript
import type { Generator, NounDef, FactTypeDef, ConstraintDef, StateMachineDef } from '../model/types'
import type { DomainModel } from '../model/domain-model'

// The output shape (same as before — this is the WASM evaluator's input)
export interface ConstraintIR {
  domain: string
  nouns: Record<string, { objectType: string; enumValues?: string[]; valueType?: string; superType?: string }>
  factTypes: Record<string, { reading: string; roles: Array<{ nounName: string; roleIndex: number }> }>
  constraints: Array<{ id: string; kind: string; modality: string; deonticOperator?: string; text: string; spans: Array<{ factTypeId: string; roleIndex: number; subsetAutofill?: boolean }>; setComparisonArgumentLength?: number; clauses?: string[]; entity?: string }>
  stateMachines: Record<string, { nounName: string; statuses: string[]; transitions: Array<{ from: string; to: string; event: string; guard?: { graphSchemaId: string; constraintIds: string[] } }> }>
}

// Direct generator — uses DomainModel accessors to serialize to IR shape
export async function generateConstraintIR(model: DomainModel): Promise<ConstraintIR> {
  const nouns = await model.nouns()
  const factTypes = await model.factTypes()
  const constraints = await model.constraints()
  const stateMachines = await model.stateMachines()

  return {
    domain: model.domainId,  // expose domainId as readonly property
    nouns: Object.fromEntries([...nouns].map(([name, n]) => [name, {
      objectType: n.objectType,
      ...(n.enumValues && { enumValues: n.enumValues }),
      ...(n.valueType && { valueType: n.valueType }),
      ...(n.superType && { superType: typeof n.superType === 'string' ? n.superType : n.superType.name }),
    }])),
    factTypes: Object.fromEntries([...factTypes].map(([id, ft]) => [id, {
      reading: ft.reading,
      roles: ft.roles.map(r => ({ nounName: r.nounName, roleIndex: r.roleIndex })),
    }])),
    constraints: constraints.map(c => ({
      id: c.id, kind: c.kind, modality: c.modality, text: c.text,
      spans: c.spans.map(s => ({ factTypeId: s.factTypeId, roleIndex: s.roleIndex, ...(s.subsetAutofill && { subsetAutofill: true }) })),
      ...(c.deonticOperator && { deonticOperator: c.deonticOperator }),
      ...(c.setComparisonArgumentLength && { setComparisonArgumentLength: c.setComparisonArgumentLength }),
      ...(c.clauses && { clauses: c.clauses }),
      ...(c.entity && { entity: c.entity }),
    })),
    stateMachines: Object.fromEntries([...stateMachines].map(([id, sm]) => [id, {
      nounName: sm.nounName,
      statuses: sm.statuses.map(s => s.name),
      transitions: sm.transitions.map(t => ({
        from: t.from, to: t.to, event: t.event,
        ...(t.guard && { guard: t.guard }),
      })),
    }])),
  }
}
```

- [ ] **Step 2: Update tests to use mock DomainModel**

Rewrite `constraint-ir.test.ts` to use `createMockModel()` from `src/model/test-utils.ts`. Replace `mockDB(data)` + `generateConstraintIR(db, 'd1')` with `createMockModel(typedData)` + `generateConstraintIR(model)`.

The test assertions (checking IR output shape, noun metadata, constraint spans, deontic operators, state machine transitions, guard resolution) remain the same — only the setup changes.

Use the factory functions from test-utils: `mkNounDef`, `mkValueNounDef`, `mkFactType`, `mkConstraint`, `mkStateMachine`.

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/generate/constraint-ir.test.ts`
Expected: PASS — same output shape, different internals

- [ ] **Step 4: Commit**

```bash
git add src/generate/constraint-ir.ts src/generate/constraint-ir.test.ts
git commit -m "refactor(generate): rewrite constraint-ir as DomainModel consumer"
```

---

### Task 11: Rewrite OpenAPI Generator (Direct)

**Files:**
- Modify: `src/generate/openapi.ts`
- Modify: `src/generate/openapi.test.ts`

This is the most complex generator. It uses DomainModel accessors directly for its three-pass constraint-span processing.

- [ ] **Step 1: Rewrite generator to accept DomainModel**

Change signature from `generateOpenAPI(db, domainId)` to `generateOpenAPI(model: DomainModel)`.

Replace all `fetchAll(db, collection, filter)` calls with DomainModel accessors:
- `db.findInCollection('nouns', ...)` → `model.nouns()` (returns Map<name, NounDef>)
- `db.findInCollection('graph-schemas', ...)` → `model.factTypes()` (graph schemas ARE fact types in the model)
- `db.findInCollection('roles', { graphSchema: ... })` → already resolved in `FactTypeDef.roles`
- `db.findInCollection('readings', { graphSchema: ... })` → already resolved in `FactTypeDef.reading`
- `db.findInCollection('constraint-spans', ...)` → `model.constraintSpans()`
- `db.findInCollection('constraints', ...)` → `model.constraints()`

The three-pass processing logic remains the same:
1. `processBinarySchemas` — single-role UCs
2. `processArraySchemas` — compound UCs
3. `processUnarySchemas` — single-role fact types

The allOf flattening logic remains the same.

Key changes:
- Nouns are now `NounDef` objects (not raw DB rows) — pass directly to fact-processors and schema-builder
- Constraint spans are already grouped by constraint ID in `model.constraintSpans()`
- Roles are already resolved in `FactTypeDef.roles` — no need to populate them manually
- Readings are already resolved in `FactTypeDef.reading` — no need for separate query

The `ensureTableExists` calls for domain-scoped entity nouns with permissions remain — iterate `model.nouns()`, filter by `objectType === 'entity'` and `permissions?.length > 0`.

- [ ] **Step 2: Update tests to use mock DomainModel**

Rewrite `openapi.test.ts`:
- Replace `mockDB(data)` with `createMockModel(typedData)`
- Replace `generateOpenAPI(db, domainId)` with `generateOpenAPI(model)`
- Update test fixtures to use NounDef types
- All assertions about output shape remain unchanged

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/generate/openapi.test.ts`
Expected: PASS

- [ ] **Step 4: Run fact-processors and schema-builder tests too**

Run: `npx vitest run src/generate/fact-processors.test.ts src/generate/schema-builder.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/generate/openapi.ts src/generate/openapi.test.ts
git commit -m "refactor(generate): rewrite OpenAPI generator for DomainModel"
```

---

### Task 12: Rewrite Readings Generator (Walker)

**Files:**
- Modify: `src/generate/readings.ts`
- Modify: `src/generate/readings.test.ts`

- [ ] **Step 1: Rewrite as direct generator using DomainModel**

Change signature from `generateReadings(db, domainId)` to `generateReadings(model: DomainModel)`.

The readings generator reconstructs FORML2 text. It needs:
- `model.nouns()` — for entity/value declarations, superType, referenceScheme, valueType, format, pattern, enum
- `model.factTypes()` — for readings with roles
- `model.constraints()` — for constraint annotations on readings
- `model.constraintSpans()` — to link constraints to specific readings/roles
- `model.stateMachines()` — for state machine transition text

Replace all `fetchAll(db, ...)` calls with model accessors. The key simplification:

**Old pattern (manual FK resolution):**
1. Fetch nouns → build `nounById: Map<id, noun>`
2. Fetch readings → for each reading, fetch roles → look up `nounById[role.noun_id]`
3. Fetch constraints → fetch constraint_spans → look up `roleById[span.role_id]` → find graph_schema
4. Use nounById/roleById/constraintById maps to annotate readings with constraints

**New pattern (pre-resolved references):**
1. `model.nouns()` → `Map<name, NounDef>` (entity/value declarations, superType, referenceScheme)
2. `model.factTypes()` → `Map<id, FactTypeDef>` — each carries `reading` text and resolved `roles[].nounDef`
3. `model.constraints()` → `ConstraintDef[]` — each carries `spans[].factTypeId` and `spans[].roleIndex`
4. Match constraints to fact types by `span.factTypeId === factType.id` — no role/noun ID lookup needed
5. `model.stateMachines()` → `Map<id, StateMachineDef>` — statuses and transitions pre-resolved

The FORML2 text generation logic (entity declarations, reading lines, constraint annotations, state machine transitions) remains the same — only the data access layer changes.

- [ ] **Step 2: Update tests**

Rewrite `readings.test.ts` to use `createMockModel()`. All assertions about FORML2 output text remain the same.

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/generate/readings.test.ts`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/generate/readings.ts src/generate/readings.test.ts
git commit -m "refactor(generate): rewrite readings generator for DomainModel"
```

---

### Task 13: Rewrite iLayer Generator (Walker)

**Files:**
- Modify: `src/generate/ilayer.ts`
- Modify: `src/generate/ilayer.test.ts`

- [ ] **Step 1: Rewrite to accept DomainModel**

Change signature from `generateILayer(db, domainId)` to `generateILayer(model: DomainModel)`.

Replace `findInCollection` calls with model accessors:
- `model.nouns()` — entity nouns with permissions, plural, valueType, format, enum
- `model.factTypes()` — grouped by entity noun. Use readings text to classify entity→value (fields) vs entity→entity (nav links)
- `model.stateMachines()` — for action buttons on detail/edit layers

The reading parsing logic (greedy noun matching from both ends) stays the same but operates on `FactTypeDef.reading` text instead of raw reading rows.

- [ ] **Step 2: Update tests**

Rewrite `ilayer.test.ts` to use `createMockModel()`. Key fixture changes:
- `mkNoun({ permissions: [...] })` → `mkNounDef({ permissions: [...] })`
- Role/reading data now comes pre-resolved in FactTypeDef objects

All assertions about layer file generation remain unchanged.

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/generate/ilayer.test.ts`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/generate/ilayer.ts src/generate/ilayer.test.ts
git commit -m "refactor(generate): rewrite iLayer generator for DomainModel"
```

---

### Task 14: Rewrite XState Generator (Direct)

**Files:**
- Modify: `src/generate/xstate.ts`
- Modify: `src/generate/xstate.test.ts`

- [ ] **Step 1: Rewrite to accept DomainModel**

Change signature from `generateXState(db, domainId)` to `generateXState(model: DomainModel)`.

The XState generator is the second most complex (after OpenAPI). It needs:
- `model.stateMachines()` — the core data, now fully resolved with StatusDef[], TransitionDef[] including VerbDef with func
- `model.nouns()` — to resolve noun names for machine IDs
- `model.factTypes()` — for system prompt context (related schemas)
- `model.readings()` — for system prompt context (reading text)

Key simplifications:
- The deep nested verb → function resolution chain is now pre-resolved in `TransitionDef.verb.func`
- The initial state detection (status with no incoming transitions) stays the same
- System prompt generation uses `model.readings()` instead of querying readings collection
- Related schema expansion uses `model.factTypes()` roles to find noun graphs

- [ ] **Step 2: Update tests**

Rewrite `xstate.test.ts` to use `createMockModel()` and `mkStateMachine()`. All assertions about XState config JSON, agent tools, and system prompts remain unchanged.

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/generate/xstate.test.ts`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/generate/xstate.ts src/generate/xstate.test.ts
git commit -m "refactor(generate): rewrite XState generator for DomainModel"
```

---

## Chunk 3: MDXUI Generator + Integration

### Task 15: New MDXUI Generator (Walker — Proof of Architecture)

**Files:**
- Create: `src/generate/mdxui.ts`
- Create: `src/generate/mdxui.test.ts`

The mdxui generator produces MDX page files using mdxui components (from `@dot-do/mdxui`). It generates per-entity documentation and interactive UI pages.

**Output structure:**
```
{
  files: {
    "pages/{slug}.mdx": Entity detail/form page
    "pages/{slug}-list.mdx": Entity list page
    "pages/index.mdx": Index page with Cards
  }
}
```

**Component mapping from fact types:**
- Binary FT (entity→value, string) → `<TextBox />` (or `<TextArea />` for long text)
- Binary FT (entity→value, number) → `<Slider />` or text input
- Binary FT (entity→value, enum) → `<Select />` with options
- Binary FT (entity→value, boolean) → `<Checkbox />`
- Binary FT (entity→value, email format) → `<TextBox type="email" />`
- Binary FT (entity→value, date format) → `<DatePicker />`
- Binary FT (entity→entity) → navigation `<Card />` link
- Unary FT → `<Checkbox />`
- Constraints → `<Callout type="info" />` documenting the constraint

- [ ] **Step 1: Write failing tests**

```typescript
// src/generate/mdxui.test.ts
import { describe, it, expect } from 'vitest'
import { generateMdxui } from './mdxui'
import { createMockModel, mkNounDef, mkValueNounDef, mkFactType, mkConstraint } from '../model/test-utils'

describe('generateMdxui', () => {
  it('generates entity detail page with form fields from binary fact types', async () => {
    const customer = mkNounDef({ name: 'Customer', plural: 'customers', permissions: ['list', 'read', 'create'] })
    const name = mkValueNounDef({ name: 'Name', valueType: 'string' })
    const email = mkValueNounDef({ name: 'Email', valueType: 'string', format: 'email' })
    const ft1 = mkFactType({ reading: 'Customer has Name', roles: [{ nounDef: customer, roleIndex: 0 }, { nounDef: name, roleIndex: 1 }] })
    const ft2 = mkFactType({ reading: 'Customer has Email', roles: [{ nounDef: customer, roleIndex: 0 }, { nounDef: email, roleIndex: 1 }] })

    const model = createMockModel({ nouns: [customer, name, email], factTypes: [ft1, ft2] })
    const result = await generateMdxui(model)

    expect(result.files['pages/customers.mdx']).toBeDefined()
    const page = result.files['pages/customers.mdx']
    expect(page).toContain('Customer')
    expect(page).toContain('Name')
    expect(page).toContain('Email')
  })

  it('generates index page with Cards for all entities', async () => {
    const customer = mkNounDef({ name: 'Customer', plural: 'customers', permissions: ['list'] })
    const order = mkNounDef({ name: 'Order', plural: 'orders', permissions: ['list'] })

    const model = createMockModel({ nouns: [customer, order] })
    const result = await generateMdxui(model)

    expect(result.files['pages/index.mdx']).toBeDefined()
    const index = result.files['pages/index.mdx']
    expect(index).toContain('Customer')
    expect(index).toContain('Order')
  })

  it('maps value types to appropriate mdxui components', async () => {
    const entity = mkNounDef({ name: 'Task', plural: 'tasks', permissions: ['create'] })
    const priority = mkValueNounDef({ name: 'Priority', valueType: 'string', enumValues: ['low', 'medium', 'high'] })
    const active = mkValueNounDef({ name: 'Active', valueType: 'boolean' })
    const dueDate = mkValueNounDef({ name: 'DueDate', valueType: 'string', format: 'date' })

    const ft1 = mkFactType({ reading: 'Task has Priority', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: priority, roleIndex: 1 }] })
    const ft2 = mkFactType({ reading: 'Task is Active', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: active, roleIndex: 1 }] })
    const ft3 = mkFactType({ reading: 'Task has DueDate', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: dueDate, roleIndex: 1 }] })

    const model = createMockModel({ nouns: [entity, priority, active, dueDate], factTypes: [ft1, ft2, ft3] })
    const result = await generateMdxui(model)

    const page = result.files['pages/tasks.mdx']
    expect(page).toContain('Select') // enum → Select
    expect(page).toContain('Checkbox') // boolean → Checkbox
    expect(page).toContain('DatePicker') // date format → DatePicker
  })

  it('includes constraint documentation as Callouts', async () => {
    const customer = mkNounDef({ name: 'Customer', plural: 'customers', permissions: ['read'] })
    const name = mkValueNounDef({ name: 'Name', valueType: 'string' })
    const ft = mkFactType({ reading: 'Customer has Name', roles: [{ nounDef: customer, roleIndex: 0 }, { nounDef: name, roleIndex: 1 }] })
    const uc = mkConstraint({ kind: 'UC', text: 'Each Customer has at most one Name', spans: [{ factTypeId: ft.id, roleIndex: 1 }] })

    const model = createMockModel({ nouns: [customer, name], factTypes: [ft], constraints: [uc] })
    const result = await generateMdxui(model)

    const page = result.files['pages/customers.mdx']
    expect(page).toContain('Callout')
    expect(page).toContain('Each Customer has at most one Name')
  })

  it('generates entity-to-entity navigation as Card links', async () => {
    const customer = mkNounDef({ name: 'Customer', plural: 'customers', permissions: ['read'] })
    const order = mkNounDef({ name: 'Order', plural: 'orders', permissions: ['read'] })
    const ft = mkFactType({ reading: 'Customer has Order', roles: [{ nounDef: customer, roleIndex: 0 }, { nounDef: order, roleIndex: 1 }] })

    const model = createMockModel({ nouns: [customer, order], factTypes: [ft] })
    const result = await generateMdxui(model)

    const page = result.files['pages/customers.mdx']
    expect(page).toContain('Card')
    expect(page).toContain('Order')
  })

  it('skips entities without permissions', async () => {
    const hidden = mkNounDef({ name: 'Hidden', permissions: [] })
    const model = createMockModel({ nouns: [hidden] })
    const result = await generateMdxui(model)

    expect(result.files['pages/hiddens.mdx']).toBeUndefined()
  })
})
```

- [ ] **Step 2: Run tests — expect FAIL**

Run: `npx vitest run src/generate/mdxui.test.ts`
Expected: FAIL — module not found

- [ ] **Step 3: Implement mdxui generator as a walker**

This is the proof-of-architecture generator — it MUST use the `Generator<T, Out>` interface and be invoked via `model.render(mdxuiGenerator)`. This demonstrates that adding a new generator requires only implementing the renderer interfaces.

Create `src/generate/mdxui.ts`:

```typescript
import type { Generator, NounRenderer, FactTypeRenderer } from '../model/renderer'
import type { NounDef, FactTypeDef, ConstraintDef, StateMachineDef, RoleDef } from '../model/types'

export interface MdxuiOutput {
  files: Record<string, string>
}

// Each walker visit produces a MdxuiPart that the combiner merges
type MdxuiPart =
  | { type: 'entity'; slug: string; name: string; fields: string[]; navLinks: string[]; constraints: string[] }
  | { type: 'skip' }

export const mdxuiGenerator: Generator<MdxuiPart, MdxuiOutput> = {
  noun: {
    entity(noun: NounDef, factTypes: FactTypeDef[], constraints: ConstraintDef[]): MdxuiPart {
      if (!noun.permissions?.length) return { type: 'skip' }

      const fields: string[] = []
      const navLinks: string[] = []

      for (const ft of factTypes) {
        if (ft.arity === 1) {
          fields.push(`- **${ft.reading}**: Checkbox\n`)
          continue
        }
        if (ft.arity !== 2) continue
        const objectNoun = ft.roles[1]?.nounDef
        if (objectNoun?.objectType === 'value') {
          fields.push(renderField(objectNoun, ft))
        } else if (objectNoun?.objectType === 'entity') {
          const slug = toSlug(objectNoun)
          navLinks.push(`  <Card title="${objectNoun.name}" href="/pages/${slug}" />\n`)
        }
      }

      const constraintLines = constraints.map(c =>
        `<Callout type="info" title="${c.kind} Constraint">\n${c.text}\n</Callout>\n`)

      const slug = toSlug(noun)
      return { type: 'entity', slug, name: noun.name, fields, navLinks, constraints: constraintLines }
    },
    value(_noun: NounDef): MdxuiPart {
      return { type: 'skip' }
    },
  },
  // No factType renderer needed — entity() already processes its own fact types
  // No stateMachine renderer needed (could be added later for action buttons)
  combine(parts: MdxuiPart[]): MdxuiOutput {
    const files: Record<string, string> = {}
    const entities = parts.filter((p): p is Extract<MdxuiPart, { type: 'entity' }> => p.type === 'entity')

    for (const e of entities) {
      const lines: string[] = []
      lines.push(`import { Callout, Card, Cards } from 'mdxui/components'`)
      lines.push('')
      lines.push(`# ${e.name}`)
      lines.push('')
      if (e.fields.length) { lines.push('## Fields', '', ...e.fields) }
      if (e.navLinks.length) { lines.push('## Related', '', '<Cards>', ...e.navLinks, '</Cards>', '') }
      if (e.constraints.length) { lines.push('## Constraints', '', ...e.constraints) }
      files[`pages/${e.slug}.mdx`] = lines.join('\n')
    }

    // Index page
    const indexLines = [
      `import { Card, Cards } from 'mdxui/components'`,
      '', '# Domain Entities', '', '<Cards>',
      ...entities.map(e => `  <Card title="${e.name}" href="/pages/${e.slug}" />`),
      '</Cards>',
    ]
    files['pages/index.mdx'] = indexLines.join('\n')

    return { files }
  },
}

// Convenience wrapper for direct invocation
export async function generateMdxui(model: { render: (gen: Generator<MdxuiPart, MdxuiOutput>) => Promise<MdxuiOutput> }): Promise<MdxuiOutput> {
  return model.render(mdxuiGenerator)
}

// ── Helper functions used by the walker ──

function toSlug(noun: NounDef): string {
  return (noun.plural || noun.name.replace(/([a-z])([A-Z])/g, '$1-$2').toLowerCase() + 's')
    .toLowerCase().replace(/\s+/g, '-')
}

function renderField(noun: NounDef, ft: FactTypeDef): string {
  const label = noun.name
  if (noun.enumValues?.length) return `- **${label}**: Select (${noun.enumValues.join(', ')})\n`
  if (noun.valueType === 'boolean') return `- **${label}**: Checkbox\n`
  if (noun.format === 'date') return `- **${label}**: DatePicker\n`
  if (noun.format === 'email') return `- **${label}**: TextBox (email)\n`
  if (noun.valueType === 'number') return `- **${label}**: TextBox (number)\n`
  return `- **${label}**: TextBox\n`
}
```

- [ ] **Step 4: Run tests — expect PASS**

Run: `npx vitest run src/generate/mdxui.test.ts`
Expected: PASS

- [ ] **Step 5: Add to barrel export**

Update `src/generate/index.ts` to export `generateMdxui`.

- [ ] **Step 6: Commit**

```bash
git add src/generate/mdxui.ts src/generate/mdxui.test.ts src/generate/index.ts
git commit -m "feat(generate): add mdxui generator for MDX UI pages"
```

---

### Task 16: Wire Up DO generate() and Update SQLite Generator

**Files:**
- Modify: `src/do.ts`
- Modify: `src/generate/sqlite.ts` (verify no changes needed)
- Test: `src/generate/sqlite.test.ts` (verify still passes)

- [ ] **Step 1: Update DO generate() to use rewritten generators**

Now that all generators are rewritten, update the `generate()` method in `src/do.ts` to import and call the new generator functions:

```typescript
import { generateOpenAPI } from './generate/openapi'
import { generateSQLite } from './generate/sqlite'
import { generateXState } from './generate/xstate'
import { generateILayer } from './generate/ilayer'
import { generateReadings } from './generate/readings'
import { generateConstraintIR } from './generate/constraint-ir'
import { generateMdxui } from './generate/mdxui'

async generate(domainId: string, format: string): Promise<any> {
  const model = this.getModel(domainId)
  switch (format) {
    // Direct generators (complex multi-pass logic):
    case 'openapi': return generateOpenAPI(model)
    case 'sqlite': return generateSQLite(await generateOpenAPI(model))
    case 'xstate': return generateXState(model)
    // Direct generators (simpler, but don't decompose per-entity):
    case 'ilayer': return generateILayer(model)
    case 'readings': return generateReadings(model)
    case 'constraint-ir': return generateConstraintIR(model)
    // Walker generator (proof of architecture — invoked via model.render()):
    case 'mdxui': return generateMdxui(model)
    default: throw new Error(`Unknown format: ${format}`)
  }
}
```

**Note on calling conventions:** All generator wrapper functions (`generateFoo(model)`) accept a DomainModel. Walker generators internally call `model.render(fooGenerator)` inside their wrapper. Direct generators call `model.nouns()`, `model.factTypes()`, etc. directly. The call site in `generate()` doesn't distinguish — it just calls the wrapper.
```

- [ ] **Step 2: Verify SQLite generator is unchanged**

`src/generate/sqlite.ts` takes OpenAPI output as input — no DomainModel dependency. Verify it still accepts the same OpenAPI output shape.

Run: `npx vitest run src/generate/sqlite.test.ts`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/do.ts
git commit -m "feat(do): wire generate() to rewritten generators"
```

---

### Task 17: Clean Up Old Code

**Files:**
- Modify: `src/generate/openapi.ts` — remove `fetchAll` helper
- Modify: `src/generate/constraint-ir.ts` — remove `fetchAll` helper
- Modify: `src/generate/readings.ts` — remove `fetchAll` helper
- Modify: `src/generate/ilayer.ts` — remove `fetchAll` helper
- Modify: `src/generate/xstate.ts` — remove `fetchAll` helper
- Modify: `src/generate/openapi.test.ts` — remove `mockDB`, local `mkNoun`/`mkRole`/`mkReading` factories
- Modify: `src/generate/constraint-ir.test.ts` — remove `mockDB`, local factories
- Modify: `src/generate/readings.test.ts` — remove `mockDB`, local factories
- Modify: `src/generate/ilayer.test.ts` — remove `mockDB`, local `mkNoun`/`mkRole`/`mkReading` factories
- Modify: `src/generate/xstate.test.ts` — remove `mockDB`, local factories
- Modify: `src/generate/fact-processors.test.ts` — remove local factories
- Modify: `src/generate/schema-builder.test.ts` — remove local factories
- Modify: `src/generate/index.ts` — update exports

- [ ] **Step 1: Remove old fetchAll helper from each generator**

Remove the `fetchAll(db, collection, filter)` or similar helper function from these files:
- `src/generate/openapi.ts`
- `src/generate/constraint-ir.ts`
- `src/generate/readings.ts`
- `src/generate/ilayer.ts`
- `src/generate/xstate.ts`

Search for `async function fetchAll` or `function fetchAll` in each file.

- [ ] **Step 2: Remove old mockDB from test files**

Remove the `function mockDB(data)` helper from these test files:
- `src/generate/openapi.test.ts`
- `src/generate/constraint-ir.test.ts`
- `src/generate/readings.test.ts`
- `src/generate/ilayer.test.ts`
- `src/generate/xstate.test.ts`

All should now import `createMockModel` from `../model/test-utils`.

- [ ] **Step 3: Remove old factory functions from test files**

Remove local `mkNoun`, `mkRole`, `mkReading`, `mkConstraint`, `mkStatus`, `mkTransition` factory functions from these test files (all now use centralized factories from `../model/test-utils`):
- `src/generate/openapi.test.ts`
- `src/generate/constraint-ir.test.ts`
- `src/generate/readings.test.ts`
- `src/generate/ilayer.test.ts`
- `src/generate/xstate.test.ts`
- `src/generate/fact-processors.test.ts`
- `src/generate/schema-builder.test.ts`

- [ ] **Step 4: Update barrel export**

Ensure `src/generate/index.ts` exports all generators with their new signatures, including `generateMdxui`.

- [ ] **Step 5: Commit**

```bash
git add src/generate/openapi.ts src/generate/constraint-ir.ts src/generate/readings.ts src/generate/ilayer.ts src/generate/xstate.ts src/generate/openapi.test.ts src/generate/constraint-ir.test.ts src/generate/readings.test.ts src/generate/ilayer.test.ts src/generate/xstate.test.ts src/generate/fact-processors.test.ts src/generate/schema-builder.test.ts src/generate/index.ts
git commit -m "chore: remove old query logic and test helpers"
```

---

### Task 18: Full Test Suite Verification

- [ ] **Step 1: Run all TypeScript tests**

Run: `npx vitest run`
Expected: All tests pass

- [ ] **Step 2: Run type check**

Run: `npx tsc --noEmit`
Expected: No new errors (pre-existing errors in other files are OK)

- [ ] **Step 3: Run Rust tests (constraint evaluator)**

Run: `cd crates/constraint-eval && cargo test`
Expected: All 17 tests pass (no changes to Rust code)

- [ ] **Step 4: Verify no regressions in generator output**

Each generator's test suite already asserts output shape. This step verifies the full suite together:

```bash
npx vitest run src/generate/
```

Expected: All generator tests pass. If any fail, debug and fix before committing.

- [ ] **Step 5: Final commit if any fixes needed**

Review staged changes before committing:
```bash
git status
git diff --cached
git add src/model/ src/generate/ src/do.ts src/api/generate.ts
git commit -m "fix: address test failures from full suite verification"
```

Only create this commit if there are actual fixes. If all tests passed in Steps 1-4, skip this step.
