# GraphDL Integration Tests Design

## Context

GraphDL-ORM is a Thing metamodel on Payload CMS that implements ORM2/RMAP for generating OpenAPI schemas. The Generator's RMAP output previously powered the Rocket Auto sales engine. The codebase was recently migrated from Payload v2 (manual bidirectional hooks) to v3 (native join fields), but no tests existed in either repo.

We need integration tests to:
1. Document the working behavior in the old repo (payload-experiments/samuel) as a baseline spec
2. Verify the migrated repo (graphdl-orm) produces identical results
3. Catch regressions as we build out customer support workflows

## Infrastructure

- **Framework**: Vitest
- **Database**: `mongodb-memory-server` (replica set mode) — no Docker needed
- **Payload init**: `getPayload({ config })` (v3) / `payload.init()` (v2) with `PAYLOAD_DISABLE_ADMIN=true`
- **Strategy**: Full integration tests via Payload Local API — hooks fire naturally, we assert on results
- **Both repos**: Tests written for samuel first (baseline), then ported to graphdl-orm

### Test harness setup

```
test/
  vitest.setup.ts          # MongoMemoryReplSet lifecycle
  helpers/
    initPayload.ts         # Payload initialization helper
    seed.ts                # Common seed data (nouns, verbs, readings)
  collections/
    generator.test.ts      # Tier 1: RMAP pipeline
    graph-schemas.test.ts  # Tier 1: Role auto-creation, constraints
    roles.test.ts          # Tier 2: Title, constraint-span convenience
    titles.test.ts         # Tier 2: All computed titles
    bidirectional.test.ts  # Tier 2: Context.internal sync pattern
    json-examples.test.ts  # Tier 3: Example graph creation
    resources.test.ts      # Tier 3: Resource/Graph titles
```

## Test Suites

### Tier 1: Core RMAP Pipeline

#### `generator.test.ts` — Generator RMAP Output
Seed a complete schema and verify generated OpenAPI output.

**Seed data**:
- Nouns: Person (entity), Name (value/string), Age (value/integer), Order (entity), OrderNumber (value/string)
- Readings: "Person has Name", "Person has Age", "Person places Order"
- Roles: auto-created from readings (2 per binary reading)
- Constraints: UC on Person side of each binary, making Name/Age/Order properties of Person
- roleRelationship: one-to-many for Person-Order

**Assertions**:
- Person schema has `name` (string), `age` (integer) properties
- Person schema has `orders` (array of Order refs) property
- Order schema has `orderNumber` property
- Update/New/base schema variants exist with correct allOf chains
- allOf chains flatten correctly (no circular refs)
- OpenAPI paths generated for CRUD based on permissions
- WHERE clause schemas generated for Payload query syntax

#### `graph-schemas.test.ts` — Role Auto-Creation & Constraints

**Role auto-creation tests**:
- Create nouns "Customer" and "Product"
- Create a reading "Customer buys Product" linked to a graph schema
- Assert: 2 roles created — one with noun=Customer, one with noun=Product
- Assert: roles reference the correct graph schema
- Assert: reading updated with role references
- Assert: creating a second reading on same schema does NOT create duplicate roles

**Constraint creation tests** (one per cardinality):
- Setup: graph schema with 2 roles
- Set roleRelationship = 'many-to-one' → assert UC constraint span on role[0]
- Set roleRelationship = 'one-to-many' → assert UC constraint span on role[1]
- Set roleRelationship = 'many-to-many' → assert UC constraint span on both roles (same constraint)
- Set roleRelationship = 'one-to-one' → assert 2 separate UC constraints, one span each

### Tier 2: Data Integrity

#### `titles.test.ts` — Computed Title Generation
For each collection with a title hook, create a document and assert the computed title:
- GraphSchema: name or first reading text
- Role: `{noun.name} - {graphSchema.title}`
- ConstraintSpan: `{constraint.modality} {constraint.kind} - {role names} - {graphSchema.title}`
- Constraint: derived from spans
- Graph: type title with resource values substituted
- Resource: `{type.name} - {value or references}`
- Transition: `{from.name} -> {to.name} - {machineDefinition.title}`
- Status: `{name} - {stateMachineDefinition.title}`
- StateMachineDefinition: `{noun.name} - State Machine`

#### `roles.test.ts` — Constraint Span Convenience
- Add a raw constraint (not a constraint-span) to a role's constraints field
- Assert: a constraint-span was auto-created bridging the constraint to the role
- Assert: existing constraint-spans are reused, not duplicated

#### `bidirectional.test.ts` — Sync Pattern
- Create a reading with graphSchema → assert GraphSchema.readings includes it (via join/relationship)
- Create a role with graphSchema → assert GraphSchema.roles includes it
- Create a status with stateMachineDefinition → assert definition.statuses includes it
- Verify no infinite loops (context.internal guard)

### Tier 3: Instance Layer

#### `json-examples.test.ts` — Example Graph Creation
- Create a JSON example `{ "name": "Alice", "age": 30 }` for noun Person
- Assert: graphs created for Person-has-Name and Person-has-Age schemas
- Assert: resource-roles link to correct roles
- Assert: verbatim flag skips graph creation

#### `resources.test.ts` — Resource/Graph Title Computation
- Create resources with various reference types (single value, multiple references, graph references)
- Assert correct title generation with polymorphic resolution

## Verification

After implementing tests in samuel:
1. `npm test` passes — all hooks produce expected results
2. Snapshot the Generator output as a golden file for regression testing
3. Port tests to graphdl-orm, adapt for join field shapes
4. Compare generator output between repos — should be identical
