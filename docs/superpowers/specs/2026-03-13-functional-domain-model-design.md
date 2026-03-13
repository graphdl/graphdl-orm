# Functional Domain Model — Design Spec

> **For agentic workers:** This spec defines the functional programming rewrite of graphdl-orm's internal runtime semantics. The HTTP surface, DO/SQLite storage, and all existing functionality are preserved.

**Goal:** Replace ad-hoc per-generator query logic with a lazy, cached DomainModel on the DO that presents nouns as functions, fact types as higher-order functions, and constraints as predicates. Generators become renderers passed into the model — not separate pipelines.

**Architecture:** The DomainModel lives on the DO instance, lazily loads from local SQLite, caches across requests, and invalidates on writes. Generators are sets of rendering functions the model applies at each node. New generators require only new renderers — no new query logic.

---

## 1. DomainModel — Lazy Functional Core

The `DomainModel` class wraps the DO's `SqlStorage` and provides typed, cached accessors.

### Location

`src/model/domain-model.ts` — new file on the DO, instantiated once per domain.

### Interface

```typescript
class DomainModel {
  private cache: Map<string, any> = new Map()

  constructor(private sql: SqlStorage, private domainId: string) {}

  // ── Nouns as functions ──────────────────────────────────────
  // name → NounDef (type constructor)
  async nouns(): Promise<Map<string, NounDef>>
  async noun(name: string): Promise<NounDef | undefined>

  // ── Fact types as higher-order functions ─────────────────────
  // compose noun-functions into predicates
  async factTypes(): Promise<Map<string, FactTypeDef>>
  async factTypesFor(noun: NounDef): Promise<FactTypeDef[]>

  // ── Constraints as predicates ───────────────────────────────
  // Population → Violation[]
  async constraints(): Promise<ConstraintDef[]>
  async constraintsFor(factTypes: FactTypeDef[]): Promise<ConstraintDef[]>

  // ── Constraint spans (reconstructed from flat DB rows) ──────
  // Groups one-row-per-role rows into multi-role constraint spans
  async constraintSpans(): Promise<Map<string, SpanDef[]>>

  // ── State machines ──────────────────────────────────────────
  async stateMachines(): Promise<Map<string, StateMachineDef>>

  // ── Readings (raw text, for generators that parse them) ─────
  async readings(): Promise<ReadingDef[]>

  // ── Rendering ───────────────────────────────────────────────
  // Apply a renderer across the entire model — convenience walker
  async render<T, Out>(generator: Generator<T, Out>): Promise<Out>

  // ── Invalidation ────────────────────────────────────────────
  // Scoped cache invalidation; called automatically by DO write methods
  invalidate(collection?: string): void
}
```

### Types

Shared types used by all generators. Based on the existing `ConstraintIR` types from `constraint-ir.ts`, promoted to a shared module.

```typescript
// src/model/types.ts

interface NounDef {
  id: string
  name: string
  plural?: string
  description?: string
  objectType: 'entity' | 'value'
  domainId: string

  // Value type properties
  valueType?: string            // 'string' | 'number' | 'boolean'
  format?: string               // 'email' | 'date' | 'uri' | etc.
  pattern?: string              // regex pattern
  enumValues?: string[]         // parsed from JSON string in DB

  // Validation
  minimum?: number
  exclusiveMinimum?: number
  maximum?: number
  exclusiveMaximum?: number
  minLength?: number
  maxLength?: number
  multipleOf?: number

  // Entity type properties
  superType?: string            // supertype noun name
  referenceScheme?: string[]    // noun names composing the reference scheme

  // Access control
  permissions?: {
    list?: boolean
    read?: boolean
    create?: boolean
    update?: boolean
    delete?: boolean
  }
}

interface FactTypeDef {
  id: string                    // graph_schema_id
  reading: string               // "Customer has Name"
  roles: RoleDef[]
  arity: number                 // roles.length
}

interface RoleDef {
  id: string
  nounName: string
  nounDef: NounDef              // resolved reference
  roleIndex: number
}

interface ConstraintDef {
  id: string
  kind: 'UC' | 'MC' | 'RC' | 'SS' | 'XC' | 'EQ' | 'OR' | 'XO'
  modality: 'Alethic' | 'Deontic'
  deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
  text: string
  spans: SpanDef[]
  entity?: string
  clauses?: string[]
  setComparisonArgumentLength?: number  // for set comparison constraints (XC, EQ, OR, XO)
}

interface SpanDef {
  factTypeId: string
  roleIndex: number
  subsetAutofill?: boolean
}

interface StateMachineDef {
  id: string
  nounName: string
  nounDef: NounDef              // resolved noun reference
  statuses: StatusDef[]
  transitions: TransitionDef[]
}

interface StatusDef {
  id: string
  name: string
}

interface TransitionDef {
  from: string
  to: string
  event: string                 // event type name
  eventTypeId: string
  verb?: VerbDef                // optional verb executed during transition
  guard?: { graphSchemaId: string; constraintIds: string[] }
}

interface VerbDef {
  id: string
  name: string
  func?: {
    callbackUrl?: string
    httpMethod?: string
    functionType?: string
  }
}

interface ReadingDef {
  id: string
  text: string
  graphSchemaId: string
  roles: RoleDef[]              // resolved
}
```

### Caching Strategy

Each accessor checks `this.cache.get(key)` before querying SQLite. Cache keys are scoped:

| Key | Populated by | Invalidated by writes to |
|-----|-------------|-------------------------|
| `nouns` | `nouns()` | nouns |
| `factTypes` | `factTypes()` | graph-schemas, readings, roles |
| `constraints` | `constraints()` | constraints, constraint-spans |
| `constraintSpans` | `constraintSpans()` | constraints, constraint-spans |
| `stateMachines` | `stateMachines()` | state-machine-definitions, statuses, transitions, guards, event-types |
| `readings` | `readings()` | readings |

`invalidate(collection)` clears affected keys using the `INVALIDATION_MAP` (see §4). `invalidate()` clears everything.

### Lifecycle

- Instantiated in `GraphDLDB.getModel(domainId)` — one DomainModel per active domain
- Stored in a `Map<string, DomainModel>` on the DO instance
- Auto-invalidated by DO write methods (`createInCollection`, `updateInCollection`, `deleteInCollection`)
- Garbage collected when no references remain (DOs are long-lived but not permanent)

---

## 2. Renderers — Generators as Functions

A generator is a set of rendering functions + a combiner. The model walks itself, applies renderers at each node, and combines the results.

### Renderer Interfaces

```typescript
// src/model/renderer.ts

// Renders a single noun (entity or value type)
interface NounRenderer<T> {
  entity(noun: NounDef, factTypes: FactTypeDef[], constraints: ConstraintDef[]): T
  value(noun: NounDef): T
}

// Renders a single fact type
// Receives resolved roles with noun references
interface FactTypeRenderer<T> {
  // Default renderers by arity
  unary?(role: RoleDef, constraints: ConstraintDef[]): T     // checkbox/switch
  binary?(entity: RoleDef, value: RoleDef, constraints: ConstraintDef[]): T  // field
  nary?(roles: RoleDef[], constraints: ConstraintDef[]): T   // custom

  // Custom override — always takes precedence when provided
  custom?(factType: FactTypeDef, roles: RoleDef[], constraints: ConstraintDef[]): T
}

// A complete generator
interface Generator<T, Out> {
  noun: NounRenderer<T>
  factType?: FactTypeRenderer<T>
  constraint?: (constraint: ConstraintDef) => T
  stateMachine?: (sm: StateMachineDef) => T
  combine(parts: T[]): Out
}
```

### DomainModel.render()

```typescript
async render<T, Out>(gen: Generator<T, Out>): Promise<Out> {
  const parts: T[] = []
  const nouns = await this.nouns()

  for (const [name, noun] of nouns) {
    if (noun.objectType === 'entity') {
      const fts = await this.factTypesFor(noun)
      const cs = await this.constraintsFor(fts)
      parts.push(gen.noun.entity(noun, fts, cs))
    } else {
      parts.push(gen.noun.value(noun))
    }
  }

  if (gen.factType) {
    for (const [id, ft] of await this.factTypes()) {
      const cs = await this.constraintsFor([ft])
      const roles = ft.roles

      if (gen.factType.custom) {
        parts.push(gen.factType.custom(ft, roles, cs))
      } else if (ft.arity === 1 && gen.factType.unary) {
        parts.push(gen.factType.unary(roles[0], cs))
      } else if (ft.arity === 2 && gen.factType.binary) {
        parts.push(gen.factType.binary(roles[0], roles[1], cs))
      } else if (gen.factType.nary) {
        parts.push(gen.factType.nary(roles, cs))
      }
    }
  }

  if (gen.constraint) {
    for (const c of await this.constraints()) {
      parts.push(gen.constraint(c))
    }
  }

  if (gen.stateMachine) {
    for (const [, sm] of await this.stateMachines()) {
      parts.push(gen.stateMachine(sm))
    }
  }

  return gen.combine(parts)
}
```

---

## 3. Generator Rewrites

Generators come in two styles: **walker generators** that fit the `render()` walker, and **direct generators** that use DomainModel accessors for complex multi-pass logic. Both get their data from DomainModel — neither queries the DB directly.

### OpenAPI Generator (direct)

Currently: queries nouns, readings, roles, constraints, constraint-spans (5 collections). Three-pass processing: binary facts → properties, compound UCs → array types / association schemas, unary facts → booleans.

This three-tier constraint-span-driven logic doesn't fit the uniform walker. The OpenAPI generator uses DomainModel accessors directly:

```typescript
async function generateOpenAPI(model: DomainModel): Promise<OpenAPIOutput> {
  const nouns = await model.nouns()
  const factTypes = await model.factTypes()
  const constraints = await model.constraints()
  const constraintSpans = await model.constraintSpans()

  // Three-pass processing using existing fact-processors.ts + schema-builder.ts:
  // 1. processBinarySchemas — single-role UCs → typed properties
  // 2. processArraySchemas — compound UCs → array properties / association schemas
  // 3. processUnarySchemas — single-role facts → boolean properties
  // rmap.ts provides predicate parsing, property naming, noun tokenization

  return { openapi: '3.0.0', components: { schemas } }
}
```

The existing pure utility modules remain:
- **`fact-processors.ts`** (259 lines) — `processBinarySchemas`, `processArraySchemas`, `processUnarySchemas`. Refactored to accept DomainModel types instead of raw DB rows.
- **`schema-builder.ts`** (312 lines) — `createProperty`, `ensureTableExists`, `setTableProperty`. Refactored to accept `NounDef` (which now carries all the fields it needs: valueType, format, pattern, enum, min/max, referenceScheme, superType).
- **`rmap.ts`** (169 lines) — Pure functions (`toPredicate`, `findPredicateObject`, `nameToKey`, `transformPropertyName`). No changes needed — already has zero external dependencies.

### SQLite Generator (downstream of OpenAPI)

Currently: takes OpenAPI output, transforms to DDL. This remains unchanged — SQLite DDL is a pure transform of the OpenAPI schema.

```typescript
function generateSQLite(openApiOutput: OpenAPIOutput): SQLiteOutput {
  // PascalCase schema names → snake_case tables
  // camelCase properties → snake_case columns
  // $ref patterns → FK columns + indexes
  return { ddl, tableMap, fieldMap }
}
```

No DomainModel dependency. Input is OpenAPI output; output is DDL + field maps.

### iLayer Generator (walker)

Currently: queries entity nouns, readings, state machines. Builds list/detail/create/edit layers.

Rewrite: `NounRenderer<LayerDef>` — entity noun + its binary fact types → layer definitions. Unary facts → checkboxes. Uses `noun.permissions` for access control, `noun.plural` for slugs/labels, and reading text to separate field readings (entity→value) from nav readings (entity→entity). Needs state machine data for action buttons, so also uses `stateMachine` renderer.

```typescript
const ilayerGenerator: Generator<LayerPart, LayerOutput> = {
  noun: {
    entity(noun, factTypes, constraints) {
      // Binary FTs where value role is a value type → field definitions
      // Binary FTs where value role is an entity → nav relationships
      // Unary FTs → checkboxes
      // noun.permissions → which layers to generate (list/detail/create/edit)
      return { type: 'layer', noun, layers: buildLayers(noun, factTypes, constraints) }
    },
    value(noun) { return { type: 'skip' } }
  },
  stateMachine(sm) {
    // Transitions → action buttons with event names
    return { type: 'actions', nounName: sm.nounName, events: extractEvents(sm) }
  },
  combine(parts) { /* merge layers + actions by noun name */ }
}
```

### XState Generator (direct)

Currently: queries state-machine-definitions, statuses, transitions, guards, event-types, verbs, functions, nouns, roles, graph-schemas, readings. Builds XState configs + agent tool schemas + system prompts.

This generator needs broad model access beyond what the walker provides (verbs, functions, expanded role/reading context for system prompts). Uses DomainModel accessors directly:

```typescript
async function generateXState(model: DomainModel): Promise<XStateOutput> {
  const stateMachines = await model.stateMachines()  // includes StatusDef[], TransitionDef[] with VerbDef
  const nouns = await model.nouns()
  const factTypes = await model.factTypes()
  const readings = await model.readings()

  // Per state machine:
  //   1. XState config JSON (states, events, initial state)
  //   2. Agent tool schemas (unique events → tool definitions)
  //   3. System prompt (noun context + readings for related schemas)
  return { files }
}
```

### Readings Generator (walker)

Currently: queries nouns, readings, roles, constraints, state machines. Reverse-generates FORML2 text.

Rewrite: `NounRenderer<string>` + `FactTypeRenderer<string>` — each noun → declaration line, each fact type → reading line + indented constraint annotations.

### Constraint IR Generator (walker)

Currently: queries 11 collections. Produces the same typed structure as DomainModel.

Rewrite: Nearly a passthrough — the DomainModel already IS the constraint IR. The renderer serializes it to the shape expected by the WASM evaluator.

---

## 4. Cache Invalidation

Cache invalidation happens automatically inside the DO's write methods — not in hooks. Hooks use a `DurableObjectStub` (not the DO instance), so they cannot access the DomainModel directly.

```typescript
// In GraphDLDB (do.ts) — write methods auto-invalidate
async createInCollection(collection: string, data: any): Promise<any> {
  const result = await this._insertRow(collection, data)
  // Auto-invalidate any cached model for this domain
  const domainId = data.domain_id ?? data.domainId
  if (domainId) this.getModel(domainId).invalidate(collection)
  return result
}
```

The dependency map inside `invalidate(collection)` determines which cache keys to clear:

```typescript
const INVALIDATION_MAP: Record<string, string[]> = {
  'nouns':                       ['nouns', 'factTypes', 'constraints'],
  'graph-schemas':               ['factTypes'],
  'readings':                    ['factTypes', 'readings'],
  'roles':                       ['factTypes'],
  'constraints':                 ['constraints'],
  'constraint-spans':            ['constraints'],
  'state-machine-definitions':   ['stateMachines'],
  'statuses':                    ['stateMachines'],
  'transitions':                 ['stateMachines'],
  'guards':                      ['stateMachines'],
  'event-types':                 ['stateMachines'],
}
```

No hook logic changes. Hooks continue to call `db.createInCollection()` — invalidation is transparent.

---

## 5. DO Integration

```typescript
// In GraphDLDB (do.ts)
private models: Map<string, DomainModel> = new Map()

getModel(domainId: string): DomainModel {
  let model = this.models.get(domainId)
  if (!model) {
    model = new DomainModel(this.sql, domainId)
    this.models.set(domainId, model)
  }
  return model
}
```

The generate handler changes from:
```typescript
const output = await generateOpenAPI(db, domainId)
```
To:
```typescript
const model = db.getModel(domainId)

// Direct generators (complex multi-pass logic):
const openapi = await generateOpenAPI(model)
const xstate = await generateXState(model)

// Walker generators (uniform traversal):
const readings = await model.render(readingsGenerator)
const ilayer = await model.render(ilayerGenerator)
const constraintIr = await model.render(constraintIrGenerator)

// Downstream transform (no DomainModel):
const sqlite = generateSQLite(openapi)
```

---

## 6. Fact Type Rendering by Arity

Default rendering behavior based on fact type arity:

| Arity | Example | Default Widget | Default Behavior |
|-------|---------|---------------|-----------------|
| Unary | `Customer is active` | Checkbox/Switch | `(entity) => switch({ entity, fact })` |
| Binary | `Customer has Name` | Field | `(entity, value) => field({ entity, field: value })` |
| Ternary+ | `Plan charges Price per Interval` | No default | Must provide custom renderer |

Any arity can override the default with a custom rendering function.

---

## 7. File Structure

```
src/model/
  types.ts          — shared NounDef, FactTypeDef, ConstraintDef, StateMachineDef, etc.
  domain-model.ts   — DomainModel class (lazy cache + accessors + render walker)
  renderer.ts       — Generator/Renderer interfaces

src/generate/
  openapi.ts        — generateOpenAPI(model) — direct DomainModel access (3-pass constraint logic)
  sqlite.ts         — generateSQLite(openApiOutput) — pure transform, no DomainModel dependency
  ilayer.ts         — ilayerGenerator: Generator<LayerPart, LayerOutput> — walker
  xstate.ts         — generateXState(model) — direct DomainModel access (broad data needs)
  readings.ts       — readingsGenerator: Generator<string, ReadingsOutput> — walker
  constraint-ir.ts  — constraintIrGenerator: Generator<IRPart, ConstraintIR> — walker (passthrough)
  fact-processors.ts — processBinarySchemas, processArraySchemas, processUnarySchemas (refactored for DomainModel types)
  schema-builder.ts  — createProperty, ensureTableExists, setTableProperty (refactored for NounDef)
  rmap.ts            — pure predicate parsing + property naming (unchanged)
  index.ts           — barrel exports
```

---

## 8. Migration Path

1. Create `src/model/` with types, DomainModel, and renderer interfaces
2. Add `getModel()` and auto-invalidation to the DO's write methods
3. Refactor `fact-processors.ts` and `schema-builder.ts` to accept DomainModel types instead of raw DB rows
4. Rewrite OpenAPI generator to use DomainModel accessors directly (most complex, do first)
5. SQLite generator: no changes (already a pure transform of OpenAPI output)
6. Rewrite remaining generators — walker generators (iLayer, readings, constraint-ir) and direct generator (XState)
7. Remove old ad-hoc query logic from generators
8. Existing tests pass with new generators (same output, different internals)

All existing HTTP endpoints, CRUD operations, hooks, and tests remain unchanged. Only the internal path from "generate request" to "output" changes.

---

## 9. What Does NOT Change

- HTTP surface (all 12 endpoints)
- CRUD semantics (where/limit/sort/depth)
- DO/SQLite storage
- Hook firing and composition chain
- WASM constraint evaluator
- Collection slugs and field maps
- Parse and verify endpoints (pure functions, no model dependency)
- 308 TypeScript + 17 Rust tests

## 10. Success Criteria

- All 6 generators produce identical output (walker generators via `render()`, direct generators via DomainModel accessors)
- No generator directly queries the DB — all go through DomainModel (except SQLite, which transforms OpenAPI output)
- Adding a new walker generator requires only implementing `Generator<T, Out>`
- DomainModel caches survive across requests on the DO
- DO write methods auto-invalidate affected caches (no manual invalidation calls)
- Existing utility modules (`fact-processors.ts`, `schema-builder.ts`, `rmap.ts`) are refactored for DomainModel types
- All existing tests pass unchanged
