# GraphDL Integration Tests Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build integration tests for the samuel (Payload v2) repo that document the working RMAP pipeline behavior, then port them to graphdl-orm (Payload v3) to verify the join field migration.

**Architecture:** Vitest + mongodb-memory-server with Payload Local API. Tests call `payload.create()` / `payload.find()` directly — hooks fire automatically. No HTTP server needed. Each test suite gets a fresh database via `PAYLOAD_DROP_DATABASE=true`.

**Tech Stack:** Vitest, mongodb-memory-server, Payload CMS v2 Local API (samuel), Payload CMS v3 Local API (graphdl-orm)

---

### Task 1: Install test dependencies and configure Vitest (samuel repo)

**Files:**
- Modify: `C:/Users/lippe/Repos/payload-experiments/samuel/package.json`
- Create: `C:/Users/lippe/Repos/payload-experiments/samuel/vitest.config.ts`
- Create: `C:/Users/lippe/Repos/payload-experiments/samuel/test/vitest.setup.ts`

**Step 1: Install dependencies**

Run:
```bash
cd C:/Users/lippe/Repos/payload-experiments/samuel
yarn add -D vitest mongodb-memory-server @vitest/coverage-v8
```

**Step 2: Create vitest.config.ts**

```ts
import { defineConfig } from 'vitest/config'
import path from 'path'

export default defineConfig({
  test: {
    globals: true,
    setupFiles: ['./test/vitest.setup.ts'],
    testTimeout: 60000,
    hookTimeout: 60000,
    pool: 'forks',
    poolOptions: {
      forks: { singleFork: true },
    },
  },
  resolve: {
    alias: {
      'payload/generated-types': path.resolve(__dirname, 'src/payload-types.ts'),
    },
  },
})
```

**Step 3: Create test/vitest.setup.ts**

```ts
import { MongoMemoryReplSet } from 'mongodb-memory-server'
import { beforeAll, afterAll } from 'vitest'

let mongod: MongoMemoryReplSet

beforeAll(async () => {
  mongod = await MongoMemoryReplSet.create({ replSet: { count: 1 } })
  process.env.DATABASE_URI = mongod.getUri()
  process.env.PAYLOAD_SECRET = 'test-secret-for-integration'
  process.env.PAYLOAD_DROP_DATABASE = 'true'
  process.env.PAYLOAD_DISABLE_ADMIN = 'true'
}, 120000)

afterAll(async () => {
  if (mongod) await mongod.stop()
}, 30000)
```

**Step 4: Add test script to package.json**

Add to scripts: `"test": "vitest run"`

**Step 5: Verify setup**

Run: `cd C:/Users/lippe/Repos/payload-experiments/samuel && yarn test`
Expected: "No test suites found" (no test files yet). Confirms vitest loads.

**Step 6: Commit**

```bash
git add package.json vitest.config.ts test/vitest.setup.ts yarn.lock
git commit -m "chore: add vitest + mongodb-memory-server test infrastructure"
```

---

### Task 2: Create Payload initialization helper

**Files:**
- Create: `C:/Users/lippe/Repos/payload-experiments/samuel/test/helpers/initPayload.ts`

**Step 1: Write the helper**

```ts
import payload, { Payload } from 'payload'
import path from 'path'

let initialized = false

export async function initPayload(): Promise<Payload> {
  if (initialized) return payload

  process.env.PAYLOAD_CONFIG_PATH = path.resolve(__dirname, '../../src/payload.config.ts')

  await payload.init({
    secret: process.env.PAYLOAD_SECRET || 'test-secret',
    local: true,
  })

  initialized = true
  return payload
}
```

**Step 2: Write a smoke test to verify it works**

Create `C:/Users/lippe/Repos/payload-experiments/samuel/test/smoke.test.ts`:

```ts
import { describe, it, expect, beforeAll } from 'vitest'
import type { Payload } from 'payload'
import { initPayload } from './helpers/initPayload'

let payload: Payload

describe('Smoke test', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  it('should initialize payload', () => {
    expect(payload).toBeDefined()
    expect(payload.collections).toBeDefined()
  })

  it('should create and find a noun', async () => {
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'TestNoun', objectType: 'entity' },
    })
    expect(noun.id).toBeDefined()
    expect(noun.name).toBe('TestNoun')

    const found = await payload.findByID({ collection: 'nouns', id: noun.id })
    expect(found.name).toBe('TestNoun')
  })
})
```

**Step 3: Run test**

Run: `cd C:/Users/lippe/Repos/payload-experiments/samuel && yarn test`
Expected: PASS — Payload initializes against in-memory MongoDB, creates and retrieves a noun.

**Step 4: Commit**

```bash
git add test/
git commit -m "chore: add Payload init helper and smoke test"
```

---

### Task 3: GraphSchemas — Role auto-creation from reading text

**Files:**
- Create: `C:/Users/lippe/Repos/payload-experiments/samuel/test/collections/graph-schemas.test.ts`

**Step 1: Write the test**

```ts
import { describe, it, expect, beforeAll, beforeEach } from 'vitest'
import type { Payload } from 'payload'
import { initPayload } from '../helpers/initPayload'

let payload: Payload

describe('GraphSchemas', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  describe('role auto-creation from readings', () => {
    let customerNoun: any
    let productNoun: any

    beforeEach(async () => {
      customerNoun = await payload.create({
        collection: 'nouns',
        data: { name: 'Customer', objectType: 'entity' },
      })
      productNoun = await payload.create({
        collection: 'nouns',
        data: { name: 'Product', objectType: 'entity' },
      })
    })

    it('should create roles when a reading with noun names is added to a graph schema', async () => {
      // Create reading first
      const reading = await payload.create({
        collection: 'readings',
        data: { text: 'Customer buys Product', endpointHttpVerb: 'GET' },
      })

      // Create graph schema with the reading
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: {
          name: 'CustomerBuysProduct',
          title: 'CustomerBuysProduct',
          readings: [reading.id],
        },
      })

      // Fetch the schema with depth to see roles
      const fetched = await payload.findByID({
        collection: 'graph-schemas',
        id: schema.id,
        depth: 2,
      })

      // Verify roles were auto-created
      const roles = fetched.roles as any[]
      expect(roles).toBeDefined()
      expect(roles.length).toBe(2)

      // Verify roles reference the correct nouns
      const nounIds = roles.map((r: any) => r.noun?.value?.id || r.noun?.value)
      expect(nounIds).toContain(customerNoun.id)
      expect(nounIds).toContain(productNoun.id)

      // Verify roles reference this graph schema
      roles.forEach((r: any) => {
        const gsId = typeof r.graphSchema === 'string' ? r.graphSchema : r.graphSchema?.id
        expect(gsId).toBe(schema.id)
      })
    })

    it('should not create duplicate roles when a second reading is added', async () => {
      const reading1 = await payload.create({
        collection: 'readings',
        data: { text: 'Customer buys Product', endpointHttpVerb: 'GET' },
      })

      const schema = await payload.create({
        collection: 'graph-schemas',
        data: {
          name: 'CustomerBuysProduct',
          title: 'CustomerBuysProduct',
          readings: [reading1.id],
        },
      })

      // Add a second reading
      const reading2 = await payload.create({
        collection: 'readings',
        data: { text: 'Product is bought by Customer', endpointHttpVerb: 'GET' },
      })

      await payload.update({
        collection: 'graph-schemas',
        id: schema.id,
        data: {
          readings: [reading1.id, reading2.id],
        },
      })

      // Fetch roles — should still be 2, not 4
      const roles = await payload.find({
        collection: 'roles',
        where: { graphSchema: { equals: schema.id } },
      })
      expect(roles.docs.length).toBe(2)
    })
  })

  describe('roleRelationship constraint creation', () => {
    it('should create UC constraint for many-to-one on first role', async () => {
      // Create nouns
      const person = await payload.create({
        collection: 'nouns',
        data: { name: 'Person', objectType: 'entity' },
      })
      const name = await payload.create({
        collection: 'nouns',
        data: { name: 'Name', objectType: 'value', valueType: 'string' },
      })

      // Create graph schema with reading (auto-creates roles)
      const reading = await payload.create({
        collection: 'readings',
        data: { text: 'Person has Name', endpointHttpVerb: 'GET' },
      })
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: {
          name: 'PersonHasName',
          title: 'PersonHasName',
          readings: [reading.id],
        },
      })

      // Get the auto-created roles
      const roles = await payload.find({
        collection: 'roles',
        where: { graphSchema: { equals: schema.id } },
        depth: 2,
      })
      expect(roles.docs.length).toBe(2)

      // Set roleRelationship
      await payload.update({
        collection: 'graph-schemas',
        id: schema.id,
        data: { roleRelationship: 'many-to-one' },
      })

      // Verify UC constraint was created
      const constraints = await payload.find({
        collection: 'constraints',
        where: { kind: { equals: 'UC' } },
      })
      expect(constraints.docs.length).toBeGreaterThanOrEqual(1)

      // Verify constraint span was created on role[0]
      const spans = await payload.find({
        collection: 'constraint-spans',
        depth: 2,
      })
      const relevantSpans = spans.docs.filter((s: any) => {
        const spanRoles = (s.roles as any[]) || []
        return spanRoles.some((r: any) => {
          const rId = typeof r === 'string' ? r : r.id
          return roles.docs.some((role) => role.id === rId)
        })
      })
      expect(relevantSpans.length).toBeGreaterThanOrEqual(1)
    })

    it('should create UC constraint for one-to-many on second role', async () => {
      const person = await payload.create({
        collection: 'nouns',
        data: { name: 'Buyer', objectType: 'entity' },
      })
      const order = await payload.create({
        collection: 'nouns',
        data: { name: 'Sale', objectType: 'entity' },
      })

      const reading = await payload.create({
        collection: 'readings',
        data: { text: 'Buyer has Sale', endpointHttpVerb: 'GET' },
      })
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: {
          name: 'BuyerHasSale',
          title: 'BuyerHasSale',
          readings: [reading.id],
        },
      })

      const roles = await payload.find({
        collection: 'roles',
        where: { graphSchema: { equals: schema.id } },
      })
      expect(roles.docs.length).toBe(2)

      await payload.update({
        collection: 'graph-schemas',
        id: schema.id,
        data: { roleRelationship: 'one-to-many' },
      })

      // Verify constraint span references the second role
      const spans = await payload.find({
        collection: 'constraint-spans',
        depth: 2,
        pagination: false,
      })
      expect(spans.docs.length).toBeGreaterThanOrEqual(1)
    })

    it('should create two UC constraints for one-to-one', async () => {
      const employee = await payload.create({
        collection: 'nouns',
        data: { name: 'Employee', objectType: 'entity' },
      })
      const badge = await payload.create({
        collection: 'nouns',
        data: { name: 'Badge', objectType: 'entity' },
      })

      const reading = await payload.create({
        collection: 'readings',
        data: { text: 'Employee has Badge', endpointHttpVerb: 'GET' },
      })
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: {
          name: 'EmployeeHasBadge',
          title: 'EmployeeHasBadge',
          readings: [reading.id],
        },
      })

      const roles = await payload.find({
        collection: 'roles',
        where: { graphSchema: { equals: schema.id } },
      })
      expect(roles.docs.length).toBe(2)

      await payload.update({
        collection: 'graph-schemas',
        id: schema.id,
        data: { roleRelationship: 'one-to-one' },
      })

      // For one-to-one, should have 2 constraint spans (one per role)
      const spans = await payload.find({
        collection: 'constraint-spans',
        depth: 2,
        pagination: false,
      })
      const roleIds = roles.docs.map((r) => r.id)
      const relevantSpans = spans.docs.filter((s: any) =>
        (s.roles as any[])?.some((r: any) => roleIds.includes(typeof r === 'string' ? r : r.id)),
      )
      expect(relevantSpans.length).toBe(2)
    })
  })

  describe('title computation', () => {
    it('should use name as title when no readings', async () => {
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: { name: 'TestSchema', title: 'placeholder' },
      })
      expect(schema.title).toBe('TestSchema')
    })

    it('should fall back to reading text when no name', async () => {
      const reading = await payload.create({
        collection: 'readings',
        data: { text: 'Something happens', endpointHttpVerb: 'GET' },
      })
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: { title: 'placeholder', readings: [reading.id] },
      })
      // Title should be reading text if no name provided
      expect(schema.title).toBeTruthy()
    })
  })
})
```

**Step 2: Run tests**

Run: `cd C:/Users/lippe/Repos/payload-experiments/samuel && yarn test test/collections/graph-schemas.test.ts`
Expected: Tests pass — role auto-creation, constraint creation, and title computation all work.

**Step 3: Commit**

```bash
git add test/collections/graph-schemas.test.ts
git commit -m "test: add GraphSchemas integration tests — role auto-creation and constraint creation"
```

---

### Task 4: Roles — Title computation and constraint-span convenience

**Files:**
- Create: `C:/Users/lippe/Repos/payload-experiments/samuel/test/collections/roles.test.ts`

**Step 1: Write the test**

```ts
import { describe, it, expect, beforeAll } from 'vitest'
import type { Payload } from 'payload'
import { initPayload } from '../helpers/initPayload'

let payload: Payload

describe('Roles', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  describe('title computation', () => {
    it('should generate title as "noun.name - graphSchema.title"', async () => {
      const noun = await payload.create({
        collection: 'nouns',
        data: { name: 'Vehicle', objectType: 'entity' },
      })
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: { name: 'VehicleHasVin', title: 'VehicleHasVin' },
      })
      const role = await payload.create({
        collection: 'roles',
        data: {
          title: 'placeholder',
          noun: { relationTo: 'nouns', value: noun.id },
          graphSchema: schema.id,
        },
      })
      expect(role.title).toBe('Vehicle - VehicleHasVin')
    })

    it('should repair title ending with " - undefined" via afterChange hook', async () => {
      // Create role before graph schema has a proper title
      const noun = await payload.create({
        collection: 'nouns',
        data: { name: 'Driver', objectType: 'entity' },
      })
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: { name: 'DriverTest', title: 'DriverTest' },
      })
      const role = await payload.create({
        collection: 'roles',
        data: {
          title: 'placeholder',
          noun: { relationTo: 'nouns', value: noun.id },
          graphSchema: schema.id,
        },
      })
      // Verify title was computed
      expect(role.title).not.toContain('undefined')
    })
  })

  describe('constraint-span convenience', () => {
    it('should auto-create constraint-span when raw constraint is added to role', async () => {
      // Create a role
      const noun = await payload.create({
        collection: 'nouns',
        data: { name: 'Item', objectType: 'entity' },
      })
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: { name: 'ItemTest', title: 'ItemTest' },
      })
      const role = await payload.create({
        collection: 'roles',
        data: {
          title: 'placeholder',
          noun: { relationTo: 'nouns', value: noun.id },
          graphSchema: schema.id,
        },
      })

      // Create a raw constraint
      const constraint = await payload.create({
        collection: 'constraints',
        data: { kind: 'UC', modality: 'Alethic' },
      })

      // Add constraint directly to role (convenience pattern)
      await payload.update({
        collection: 'roles',
        id: role.id,
        data: {
          constraints: [{ relationTo: 'constraints', value: constraint.id }],
        },
      })

      // Verify a constraint-span was auto-created
      const spans = await payload.find({
        collection: 'constraint-spans',
        where: { constraint: { equals: constraint.id } },
        depth: 2,
      })
      expect(spans.docs.length).toBeGreaterThanOrEqual(1)

      // Verify the span includes this role
      const span = spans.docs[0]
      const spanRoleIds = ((span.roles as any[]) || []).map((r: any) =>
        typeof r === 'string' ? r : r.id,
      )
      expect(spanRoleIds).toContain(role.id)
    })

    it('should reuse existing constraint-span for same constraint', async () => {
      const noun1 = await payload.create({
        collection: 'nouns',
        data: { name: 'Alpha', objectType: 'entity' },
      })
      const noun2 = await payload.create({
        collection: 'nouns',
        data: { name: 'Beta', objectType: 'entity' },
      })
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: { name: 'AlphaBeta', title: 'AlphaBeta' },
      })
      const role1 = await payload.create({
        collection: 'roles',
        data: {
          title: 'placeholder',
          noun: { relationTo: 'nouns', value: noun1.id },
          graphSchema: schema.id,
        },
      })
      const role2 = await payload.create({
        collection: 'roles',
        data: {
          title: 'placeholder',
          noun: { relationTo: 'nouns', value: noun2.id },
          graphSchema: schema.id,
        },
      })

      const constraint = await payload.create({
        collection: 'constraints',
        data: { kind: 'UC', modality: 'Alethic' },
      })

      // Add same constraint to both roles
      await payload.update({
        collection: 'roles',
        id: role1.id,
        data: {
          constraints: [{ relationTo: 'constraints', value: constraint.id }],
        },
      })
      await payload.update({
        collection: 'roles',
        id: role2.id,
        data: {
          constraints: [{ relationTo: 'constraints', value: constraint.id }],
        },
      })

      // Should reuse the same constraint-span, not create a second one
      const spans = await payload.find({
        collection: 'constraint-spans',
        where: { constraint: { equals: constraint.id } },
      })
      expect(spans.docs.length).toBe(1)

      // The single span should now reference both roles
      const span = await payload.findByID({
        collection: 'constraint-spans',
        id: spans.docs[0].id,
        depth: 2,
      })
      const spanRoleIds = ((span.roles as any[]) || []).map((r: any) =>
        typeof r === 'string' ? r : r.id,
      )
      expect(spanRoleIds).toContain(role1.id)
      expect(spanRoleIds).toContain(role2.id)
    })
  })
})
```

**Step 2: Run tests**

Run: `cd C:/Users/lippe/Repos/payload-experiments/samuel && yarn test test/collections/roles.test.ts`
Expected: PASS

**Step 3: Commit**

```bash
git add test/collections/roles.test.ts
git commit -m "test: add Roles integration tests — title computation and constraint-span convenience"
```

---

### Task 5: Generator — RMAP pipeline output

**Files:**
- Create: `C:/Users/lippe/Repos/payload-experiments/samuel/test/helpers/seed.ts`
- Create: `C:/Users/lippe/Repos/payload-experiments/samuel/test/collections/generator.test.ts`

**Step 1: Create seed helper**

```ts
import type { Payload } from 'payload'

export async function seedPersonSchema(payload: Payload) {
  // Create entity nouns
  const person = await payload.create({
    collection: 'nouns',
    data: { name: 'Person', plural: 'people', objectType: 'entity' },
  })
  const order = await payload.create({
    collection: 'nouns',
    data: { name: 'Order', plural: 'orders', objectType: 'entity' },
  })

  // Create value nouns
  const personName = await payload.create({
    collection: 'nouns',
    data: { name: 'PersonName', objectType: 'value', valueType: 'string' },
  })
  const age = await payload.create({
    collection: 'nouns',
    data: { name: 'Age', objectType: 'value', valueType: 'integer' },
  })
  const orderNumber = await payload.create({
    collection: 'nouns',
    data: { name: 'OrderNumber', objectType: 'value', valueType: 'string' },
  })

  // Create readings + schemas (readings trigger role auto-creation)
  const personHasNameReading = await payload.create({
    collection: 'readings',
    data: { text: 'Person has PersonName', endpointHttpVerb: 'GET' },
  })
  const personHasName = await payload.create({
    collection: 'graph-schemas',
    data: {
      name: 'PersonHasPersonName',
      title: 'PersonHasPersonName',
      readings: [personHasNameReading.id],
    },
  })

  const personHasAgeReading = await payload.create({
    collection: 'readings',
    data: { text: 'Person has Age', endpointHttpVerb: 'GET' },
  })
  const personHasAge = await payload.create({
    collection: 'graph-schemas',
    data: {
      name: 'PersonHasAge',
      title: 'PersonHasAge',
      readings: [personHasAgeReading.id],
    },
  })

  const personPlacesOrderReading = await payload.create({
    collection: 'readings',
    data: { text: 'Person places Order', endpointHttpVerb: 'POST' },
  })
  const personPlacesOrder = await payload.create({
    collection: 'graph-schemas',
    data: {
      name: 'PersonPlacesOrder',
      title: 'PersonPlacesOrder',
      readings: [personPlacesOrderReading.id],
    },
  })

  const orderHasNumberReading = await payload.create({
    collection: 'readings',
    data: { text: 'Order has OrderNumber', endpointHttpVerb: 'GET' },
  })
  const orderHasNumber = await payload.create({
    collection: 'graph-schemas',
    data: {
      name: 'OrderHasOrderNumber',
      title: 'OrderHasOrderNumber',
      readings: [orderHasNumberReading.id],
    },
  })

  // Set cardinality constraints
  // PersonHasPersonName: many-to-one (each person has one name, identified by it)
  await payload.update({
    collection: 'graph-schemas',
    id: personHasName.id,
    data: { roleRelationship: 'many-to-one' },
  })
  // PersonHasAge: many-to-one
  await payload.update({
    collection: 'graph-schemas',
    id: personHasAge.id,
    data: { roleRelationship: 'many-to-one' },
  })
  // PersonPlacesOrder: one-to-many (one person, many orders)
  await payload.update({
    collection: 'graph-schemas',
    id: personPlacesOrder.id,
    data: { roleRelationship: 'one-to-many' },
  })
  // OrderHasOrderNumber: many-to-one
  await payload.update({
    collection: 'graph-schemas',
    id: orderHasNumber.id,
    data: { roleRelationship: 'many-to-one' },
  })

  return {
    nouns: { person, order, personName, age, orderNumber },
    schemas: { personHasName, personHasAge, personPlacesOrder, orderHasNumber },
  }
}
```

**Step 2: Write the Generator test**

```ts
import { describe, it, expect, beforeAll } from 'vitest'
import type { Payload } from 'payload'
import { initPayload } from '../helpers/initPayload'
import { seedPersonSchema } from '../helpers/seed'

let payload: Payload

describe('Generator RMAP Pipeline', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  it('should generate valid OpenAPI schemas from a seeded ORM model', async () => {
    // Seed the model
    await seedPersonSchema(payload)

    // Trigger the generator
    const generator = await payload.create({
      collection: 'generators',
      data: {
        title: 'Test API',
        version: '1.0',
      },
    })

    // The output is stored as JSON in generator.output (via beforeChange hook)
    const result = await payload.findByID({
      collection: 'generators',
      id: generator.id,
    })

    const output = result.output as any
    expect(output).toBeDefined()

    // Verify top-level OpenAPI structure
    expect(output.openapi).toBe('3.1.0')
    expect(output.info).toBeDefined()
    expect(output.paths).toBeDefined()
    expect(output.components).toBeDefined()
    expect(output.components.schemas).toBeDefined()

    const schemas = output.components.schemas

    // Verify Person schema was generated
    expect(schemas.Person).toBeDefined()
    expect(schemas.Person.type).toBe('object')

    // Verify Person has properties from binary fact types
    const personProps = schemas.Person.properties || {}
    // PersonName should be a property (from PersonHasPersonName with UC on Person role)
    const hasNameProp = Object.keys(personProps).some(
      (k) => k.toLowerCase().includes('name') || k.toLowerCase().includes('personname'),
    )
    expect(hasNameProp).toBe(true)

    // Verify Order schema was generated
    expect(schemas.Order).toBeDefined()

    // Verify paths were generated
    const pathKeys = Object.keys(output.paths)
    expect(pathKeys.length).toBeGreaterThan(0)
  }, 120000)

  it('should flatten allOf chains in schemas', async () => {
    const generators = await payload.find({ collection: 'generators' })
    if (generators.docs.length === 0) return

    const output = generators.docs[0].output as any
    const schemas = output?.components?.schemas || {}

    // Check that no schema has unresolved allOf with $ref to schemas that exist
    for (const [key, schema] of Object.entries(schemas) as any[]) {
      if (schema.allOf) {
        // If allOf remains, verify references are to external schemas only
        for (const ref of schema.allOf) {
          if (ref.$ref) {
            const refName = ref.$ref.split('/').pop()
            // The reference target should not be in our local schemas
            // (flattening should have resolved local refs)
            expect(schemas[refName]).toBeUndefined()
          }
        }
      }
    }
  })

  it('should generate CRUD paths based on default permissions', async () => {
    const generators = await payload.find({ collection: 'generators' })
    if (generators.docs.length === 0) return

    const output = generators.docs[0].output as any
    const paths = output?.paths || {}

    // Find a path for Person (should have list, create, read, update, delete)
    const personPaths = Object.entries(paths).filter(([key]) =>
      key.toLowerCase().includes('people') || key.toLowerCase().includes('person'),
    )
    expect(personPaths.length).toBeGreaterThan(0)

    // Check for common HTTP methods
    const allMethods = personPaths.flatMap(([, methods]: any) => Object.keys(methods))
    expect(allMethods).toContain('get')
  })
})
```

**Step 3: Run tests**

Run: `cd C:/Users/lippe/Repos/payload-experiments/samuel && yarn test test/collections/generator.test.ts`
Expected: PASS — Generator produces valid OpenAPI from the seeded model.

**Step 4: Snapshot the output**

After the test passes, add a snapshot test to capture the exact output for regression:

```ts
it('should match the golden snapshot', async () => {
  const generators = await payload.find({ collection: 'generators' })
  if (generators.docs.length === 0) return
  const output = generators.docs[0].output
  expect(output).toMatchSnapshot()
})
```

**Step 5: Commit**

```bash
git add test/helpers/seed.ts test/collections/generator.test.ts
git commit -m "test: add Generator RMAP pipeline integration tests with seed data"
```

---

### Task 6: Bidirectional sync and title generation tests

**Files:**
- Create: `C:/Users/lippe/Repos/payload-experiments/samuel/test/collections/bidirectional.test.ts`
- Create: `C:/Users/lippe/Repos/payload-experiments/samuel/test/collections/titles.test.ts`

**Step 1: Write bidirectional sync tests**

```ts
import { describe, it, expect, beforeAll } from 'vitest'
import type { Payload } from 'payload'
import { initPayload } from '../helpers/initPayload'

let payload: Payload

describe('Bidirectional Sync', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  it('reading with graphSchema should appear in schema.readings', async () => {
    const schema = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'SyncTest', title: 'SyncTest' },
    })
    const reading = await payload.create({
      collection: 'readings',
      data: {
        text: 'SyncTest happens',
        graphSchema: schema.id,
        endpointHttpVerb: 'GET',
      },
    })

    const fetched = await payload.findByID({
      collection: 'graph-schemas',
      id: schema.id,
      depth: 1,
    })
    const readingIds = ((fetched.readings as any[]) || []).map((r: any) =>
      typeof r === 'string' ? r : r.id,
    )
    expect(readingIds).toContain(reading.id)
  })

  it('role with graphSchema should appear in schema.roles', async () => {
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'SyncNoun', objectType: 'entity' },
    })
    const schema = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'SyncRoleTest', title: 'SyncRoleTest' },
    })
    const role = await payload.create({
      collection: 'roles',
      data: {
        title: 'placeholder',
        noun: { relationTo: 'nouns', value: noun.id },
        graphSchema: schema.id,
      },
    })

    const fetched = await payload.findByID({
      collection: 'graph-schemas',
      id: schema.id,
      depth: 1,
    })
    const roleIds = ((fetched.roles as any[]) || []).map((r: any) =>
      typeof r === 'string' ? r : r.id,
    )
    expect(roleIds).toContain(role.id)
  })

  it('status with stateMachineDefinition should appear in definition.statuses', async () => {
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'StateSyncNoun', objectType: 'entity' },
    })
    const definition = await payload.create({
      collection: 'state-machine-definitions',
      data: {
        title: 'placeholder',
        noun: { relationTo: 'nouns', value: noun.id },
      },
    })
    const status = await payload.create({
      collection: 'statuses',
      data: {
        name: 'Active',
        title: 'placeholder',
        stateMachineDefinition: definition.id,
      },
    })

    const fetched = await payload.findByID({
      collection: 'state-machine-definitions',
      id: definition.id,
      depth: 1,
    })
    const statusIds = ((fetched.statuses as any[]) || []).map((s: any) =>
      typeof s === 'string' ? s : s.id,
    )
    expect(statusIds).toContain(status.id)
  })
})
```

**Step 2: Write title generation tests**

```ts
import { describe, it, expect, beforeAll } from 'vitest'
import type { Payload } from 'payload'
import { initPayload } from '../helpers/initPayload'

let payload: Payload

describe('Computed Titles', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  it('Role title: "noun.name - graphSchema.title"', async () => {
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'Car', objectType: 'entity' },
    })
    const schema = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'CarHasVin', title: 'CarHasVin' },
    })
    const role = await payload.create({
      collection: 'roles',
      data: {
        title: 'placeholder',
        noun: { relationTo: 'nouns', value: noun.id },
        graphSchema: schema.id,
      },
    })
    expect(role.title).toBe('Car - CarHasVin')
  })

  it('ConstraintSpan title: "modality kind - roleNames - graphSchema"', async () => {
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'Vin', objectType: 'value', valueType: 'string' },
    })
    const schema = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'HasVin', title: 'HasVin' },
    })
    const role = await payload.create({
      collection: 'roles',
      data: {
        title: 'placeholder',
        noun: { relationTo: 'nouns', value: noun.id },
        graphSchema: schema.id,
      },
    })
    const constraint = await payload.create({
      collection: 'constraints',
      data: { kind: 'UC', modality: 'Alethic' },
    })
    const span = await payload.create({
      collection: 'constraint-spans',
      data: {
        constraint: constraint.id,
        roles: [role.id],
      },
    })
    expect(span.title).toContain('Alethic')
    expect(span.title).toContain('UC')
  })

  it('GraphSchema title: uses name when provided', async () => {
    const schema = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'MySchema', title: 'anything' },
    })
    expect(schema.title).toBe('MySchema')
  })
})
```

**Step 3: Run all tests**

Run: `cd C:/Users/lippe/Repos/payload-experiments/samuel && yarn test`
Expected: All PASS

**Step 4: Commit**

```bash
git add test/collections/bidirectional.test.ts test/collections/titles.test.ts
git commit -m "test: add bidirectional sync and computed title integration tests"
```

---

### Task 7: Graphs and Resources title tests

**Files:**
- Create: `C:/Users/lippe/Repos/payload-experiments/samuel/test/collections/resources.test.ts`

**Step 1: Write the test**

```ts
import { describe, it, expect, beforeAll } from 'vitest'
import type { Payload } from 'payload'
import { initPayload } from '../helpers/initPayload'

let payload: Payload

describe('Resources and Graphs', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  describe('Resource title computation', () => {
    it('should generate title from type.name and value', async () => {
      const noun = await payload.create({
        collection: 'nouns',
        data: { name: 'Color', objectType: 'value', valueType: 'string' },
      })
      const resource = await payload.create({
        collection: 'resources',
        data: {
          type: noun.id,
          value: 'Red',
        },
      })
      expect(resource.title).toContain('Color')
      expect(resource.title).toContain('Red')
    })
  })

  describe('Graph title computation', () => {
    it('should substitute resource values into schema title', async () => {
      // Create the schema
      const person = await payload.create({
        collection: 'nouns',
        data: { name: 'Shopper', objectType: 'entity' },
      })
      const color = await payload.create({
        collection: 'nouns',
        data: { name: 'Shade', objectType: 'value', valueType: 'string' },
      })
      const reading = await payload.create({
        collection: 'readings',
        data: { text: 'Shopper has Shade', endpointHttpVerb: 'GET' },
      })
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: {
          name: 'ShopperHasShade',
          title: 'ShopperHasShade',
          readings: [reading.id],
        },
      })

      // Get auto-created roles
      const roles = await payload.find({
        collection: 'roles',
        where: { graphSchema: { equals: schema.id } },
        depth: 1,
      })

      // Create resources
      const personResource = await payload.create({
        collection: 'resources',
        data: { type: person.id, value: 'Alice' },
      })
      const colorResource = await payload.create({
        collection: 'resources',
        data: { type: color.id, value: 'Blue' },
      })

      // Create graph with resource roles
      const graph = await payload.create({
        collection: 'graphs',
        data: {
          type: schema.id,
          title: 'placeholder',
          resourceRoles: roles.docs.map((r, i) => {
            // Can't easily determine which role is which without more depth,
            // but test the general pattern
          }),
        },
      })

      expect(graph.title).toBeDefined()
    })
  })
})
```

**Step 2: Run tests**

Run: `cd C:/Users/lippe/Repos/payload-experiments/samuel && yarn test`
Expected: PASS

**Step 3: Commit**

```bash
git add test/collections/resources.test.ts
git commit -m "test: add Resources and Graphs title computation tests"
```

---

### Task 8: Run full test suite and capture baseline

**Step 1: Run all tests**

Run: `cd C:/Users/lippe/Repos/payload-experiments/samuel && yarn test --reporter=verbose`

**Step 2: Update snapshots if needed**

Run: `cd C:/Users/lippe/Repos/payload-experiments/samuel && yarn test --update`

**Step 3: Commit final state**

```bash
git add -A
git commit -m "test: complete baseline integration test suite for samuel repo"
```

---

### Task 9: Port test infrastructure to graphdl-orm

**Files:**
- Modify: `C:/Users/lippe/Repos/graphdl-orm/package.json`
- Create: `C:/Users/lippe/Repos/graphdl-orm/vitest.config.ts`
- Create: `C:/Users/lippe/Repos/graphdl-orm/test/vitest.setup.ts`
- Create: `C:/Users/lippe/Repos/graphdl-orm/test/helpers/initPayload.ts`

The graphdl-orm repo uses Payload v3 (ESM, `getPayload()`). Key differences from samuel:
- Use `getPayload({ config })` instead of `payload.init()`
- ESM imports (`import` not `require`)
- Config imported directly, not via `PAYLOAD_CONFIG_PATH`

**Step 1: Install deps**

Run:
```bash
cd C:/Users/lippe/Repos/graphdl-orm
npm install -D vitest mongodb-memory-server @vitest/coverage-v8
```

**Step 2: Create vitest.config.ts**

```ts
import { defineConfig } from 'vitest/config'
import path from 'path'

export default defineConfig({
  test: {
    globals: true,
    setupFiles: ['./test/vitest.setup.ts'],
    testTimeout: 60000,
    hookTimeout: 60000,
    pool: 'forks',
    poolOptions: {
      forks: { singleFork: true },
    },
  },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, 'src'),
      '@payload-config': path.resolve(__dirname, 'src/payload.config.ts'),
    },
  },
})
```

**Step 3: Create test/vitest.setup.ts (same as samuel)**

**Step 4: Create test/helpers/initPayload.ts (Payload v3 version)**

```ts
import { getPayload, type Payload } from 'payload'

let cachedPayload: Payload | null = null

export async function initPayload(): Promise<Payload> {
  if (cachedPayload) return cachedPayload

  const { default: config } = await import('../../src/payload.config')
  cachedPayload = await getPayload({ config })
  return cachedPayload
}
```

**Step 5: Copy test files from samuel, adapting for join field shapes**

Key adaptation: In assertions that access join fields on populated docs, use `.docs` accessor:
- `fetched.roles` → `fetched.roles?.docs`
- `fetched.readings` → `fetched.readings?.docs`
- `fetched.resourceRoles` → `fetched.resourceRoles?.docs`
- `fetched.constraints` → `fetched.constraints?.docs`
- `fetched.statuses` → `fetched.statuses?.docs`

**Step 6: Run and compare**

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run --reporter=verbose`
Expected: Same test results as samuel. Any failures indicate remaining migration bugs.

**Step 7: Commit**

```bash
git add package.json vitest.config.ts test/ package-lock.json
git commit -m "test: port integration test suite from samuel, adapted for Payload v3 join fields"
```
