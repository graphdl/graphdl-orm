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

  // ── State machines ──────────────────────────────────────────
  async stateMachines(): Promise<Map<string, StateMachineDef>>

  // ── Rendering ───────────────────────────────────────────────
  // Apply a renderer across the entire model
  async render<T, Out>(generator: Generator<T, Out>): Promise<Out>

  // ── Invalidation ────────────────────────────────────────────
  // Scoped cache invalidation; called by hooks after writes
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
  objectType: 'entity' | 'value'
  enumValues?: string[]
  valueType?: string
  superType?: string
  domainId: string
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
}

interface SpanDef {
  factTypeId: string
  roleIndex: number
  subsetAutofill?: boolean
}

interface StateMachineDef {
  id: string
  nounName: string
  statuses: string[]
  transitions: TransitionDef[]
}

interface TransitionDef {
  from: string
  to: string
  event: string
  guard?: { graphSchemaId: string; constraintIds: string[] }
}
```

### Caching Strategy

Each accessor checks `this.cache.get(key)` before querying SQLite. Cache keys are scoped:

| Key | Populated by | Invalidated by writes to |
|-----|-------------|-------------------------|
| `nouns` | `nouns()` | nouns |
| `factTypes` | `factTypes()` | graph-schemas, readings, roles |
| `constraints` | `constraints()` | constraints, constraint-spans |
| `stateMachines` | `stateMachines()` | state-machine-definitions, statuses, transitions, guards |

`invalidate(collection)` clears the affected keys using a dependency map. `invalidate()` clears everything.

### Lifecycle

- Instantiated in `GraphDLDB.ensureModel(domainId)` — one DomainModel per active domain
- Stored in a `Map<string, DomainModel>` on the DO instance
- Invalidated by hooks after writes (same domain only)
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

Each existing generator becomes a `Generator<T, Out>` implementation. The query logic disappears — it's handled by the model.

### OpenAPI Generator

Currently: queries nouns, readings, roles, constraints, constraint-spans (5 collections). Builds schemas by iterating entity nouns, grouping facts, inferring property types.

Rewrite:
```typescript
const openApiGenerator: Generator<OpenAPIPart, OpenAPIOutput> = {
  noun: {
    entity(noun, factTypes, constraints) {
      // Binary fact types → properties
      // Unary fact types → boolean properties
      // Constraints → required fields, enums
      return { type: 'schema', name: noun.name, schema: buildSchema(noun, factTypes, constraints) }
    },
    value(noun) {
      return { type: 'value', name: noun.name, schema: buildValueSchema(noun) }
    }
  },
  combine(parts) {
    const schemas = Object.fromEntries(parts.filter(p => p.type === 'schema').map(p => [p.name, p.schema]))
    return { openapi: '3.0.0', components: { schemas } }
  }
}
```

### SQLite Generator

Currently: takes OpenAPI output, transforms to DDL. With the model, it can go directly.

Rewrite: `NounRenderer<DDLStatement>` — each entity noun → CREATE TABLE, binary facts → columns, constraints → SQL constraints.

### iLayer Generator

Currently: queries entity nouns, builds list/detail/create/edit layers with field type mapping.

Rewrite: `NounRenderer<LayerDef>` — entity noun + its binary fact types → layer definitions. Unary facts → checkboxes. The rendering function IS the layer definition.

### XState Generator

Currently: queries state-machine-definitions, statuses, transitions, guards. Builds XState configs.

Rewrite: Uses `stateMachine` renderer — each StateMachineDef → XState config JSON.

### Readings Generator

Currently: queries nouns, readings, roles, constraints. Reverse-generates FORML2 text.

Rewrite: `NounRenderer<string>` + `FactTypeRenderer<string>` — each noun → declaration line, each fact type → reading line + indented constraints.

### Constraint IR Generator

Currently: queries 11 collections. Already produces the same typed structure as the DomainModel.

Rewrite: Nearly a passthrough — the DomainModel already IS the constraint IR. The renderer just serializes it.

---

## 4. Hook Integration

Hooks continue to fire after creates. The only change: after a hook writes to the DB, it calls `model.invalidate(collection)`.

```typescript
// In readingAfterCreate:
await db.createInCollection('nouns', nounData)
model.invalidate('nouns')

await db.createInCollection('graph-schemas', schemaData)
model.invalidate('graph-schemas')
```

No hook logic changes — only the invalidation call is added.

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
const output = await model.render(openApiGenerator)
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
  types.ts          — shared NounDef, FactTypeDef, etc.
  domain-model.ts   — DomainModel class (lazy cache + render)
  renderer.ts       — Generator/Renderer interfaces

src/generate/
  openapi.ts        — openApiGenerator: Generator<OpenAPIPart, OpenAPIOutput>
  sqlite.ts         — sqliteGenerator: Generator<DDLStatement, string[]>
  ilayer.ts         — ilayerGenerator: Generator<LayerDef, LayerOutput>
  xstate.ts         — xstateGenerator: Generator<XStatePart, XStateOutput>
  readings.ts       — readingsGenerator: Generator<string, ReadingsOutput>
  constraint-ir.ts  — constraintIrGenerator: Generator<IRPart, ConstraintIR>
```

---

## 8. Migration Path

1. Create `src/model/` with types, DomainModel, and renderer interfaces
2. Add `getModel()` to the DO
3. Rewrite each generator as a `Generator<T, Out>` — one at a time, each independently testable
4. Add `invalidate()` calls to hooks
5. Remove old ad-hoc query logic from generators
6. Existing tests pass with new generators (same output, different internals)

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

- All 6 generators produce identical output through the renderer interface
- No generator directly queries the DB — all go through DomainModel
- Adding a new generator requires only implementing `Generator<T, Out>`
- DomainModel caches survive across requests on the DO
- Hook writes invalidate affected caches
- All existing tests pass unchanged
