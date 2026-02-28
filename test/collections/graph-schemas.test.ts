import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'

let payload: any

/** Extract plain string IDs from a field that may contain populated objects. */
function toIds(arr: any[]): string[] {
  return arr.map((r: any) => (typeof r === 'string' ? r : r?.id || r))
}

describe('GraphSchemas collection', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  // ---------------------------------------------------------------------------
  // Role auto-creation from readings
  // ---------------------------------------------------------------------------
  describe('role auto-creation from readings', () => {
    it('should auto-create roles for nouns found in reading text', async () => {
      // Create two nouns
      const customer = await payload.create({
        collection: 'nouns',
        data: { name: 'Customer', objectType: 'entity' },
      })
      const product = await payload.create({
        collection: 'nouns',
        data: { name: 'Product', objectType: 'entity' },
      })

      // In v3, create the graph-schema first, then create the reading
      // with graphSchema set. The afterChange hook on Readings will
      // auto-create roles.
      const gs = await payload.create({
        collection: 'graph-schemas',
        data: { name: 'CustomerBuysProduct' },
      })

      // Create a reading whose text contains both noun names, pointing to the schema
      const reading = await payload.create({
        collection: 'readings',
        data: {
          text: 'Customer buys Product',
          graphSchema: gs.id,
        },
      })

      // Fetch the graph schema — roles is a join field returning { docs: [...] }
      const fetched = await payload.findByID({
        collection: 'graph-schemas',
        id: gs.id,
        depth: 1,
      })

      const roles = fetched.roles?.docs || []
      expect(roles).toHaveLength(2)

      // Fetch the roles by their IDs
      const roleIds = toIds(roles)
      const rolesResult = await payload.find({
        collection: 'roles',
        where: { id: { in: roleIds } },
        depth: 0,
      })
      expect(rolesResult.docs).toHaveLength(2)

      const nounIds = rolesResult.docs.map((r: any) => r.noun?.value)
      expect(nounIds).toContain(customer.id)
      expect(nounIds).toContain(product.id)

      // Each role should have relationTo: 'nouns' (not 'graph-schemas')
      for (const role of rolesResult.docs) {
        expect(role.noun?.relationTo).toBe('nouns')
      }
    })

    it('should NOT create duplicate roles when a second reading is added', async () => {
      // Create nouns with unique names to avoid cross-contamination
      const seller = await payload.create({
        collection: 'nouns',
        data: { name: 'Seller', objectType: 'entity' },
      })
      const buyer = await payload.create({
        collection: 'nouns',
        data: { name: 'Buyer', objectType: 'entity' },
      })

      // Create graph schema first
      const gs = await payload.create({
        collection: 'graph-schemas',
        data: { name: 'SellerSellsToBuyer' },
      })

      // First reading — triggers role auto-creation
      const reading1 = await payload.create({
        collection: 'readings',
        data: {
          text: 'Seller sells to Buyer',
          graphSchema: gs.id,
        },
      })

      // Verify roles were created
      const afterFirst = await payload.findByID({
        collection: 'graph-schemas',
        id: gs.id,
        depth: 0,
      })
      const rolesAfterFirst = afterFirst.roles?.docs || []
      expect(rolesAfterFirst).toHaveLength(2)
      const roleIds = toIds(rolesAfterFirst)

      // Second reading with the same noun names — should NOT create duplicates
      // because the afterChange hook checks for existing roles
      const reading2 = await payload.create({
        collection: 'readings',
        data: {
          text: 'Buyer purchases from Seller',
          graphSchema: gs.id,
        },
      })

      // Roles should still be exactly the same 2
      const afterSecond = await payload.findByID({
        collection: 'graph-schemas',
        id: gs.id,
        depth: 0,
      })
      const rolesAfterSecond = afterSecond.roles?.docs || []
      expect(rolesAfterSecond).toHaveLength(2)
      expect(toIds(rolesAfterSecond).sort()).toEqual(roleIds.sort())
    })
  })

  // ---------------------------------------------------------------------------
  // Constraint creation from roleRelationship
  // ---------------------------------------------------------------------------
  describe('constraint creation from roleRelationship', () => {
    /**
     * Helper: create a fresh 2-role graph schema with unique nouns.
     * In v3: create schema, then reading with graphSchema set.
     * Returns { gs, roleIds } where roleIds is a plain string array.
     */
    async function createBinarySchema(nounA: string, nounB: string) {
      const [a, b] = await Promise.all([
        payload.create({ collection: 'nouns', data: { name: nounA, objectType: 'entity' } }),
        payload.create({ collection: 'nouns', data: { name: nounB, objectType: 'entity' } }),
      ])

      const gs = await payload.create({
        collection: 'graph-schemas',
        data: { name: `${nounA}Relates${nounB}` },
      })

      const reading = await payload.create({
        collection: 'readings',
        data: {
          text: `${nounA} relates ${nounB}`,
          graphSchema: gs.id,
        },
      })

      // Re-fetch to get the join field populated
      const fetched = await payload.findByID({
        collection: 'graph-schemas',
        id: gs.id,
        depth: 0,
      })
      const roles = fetched.roles?.docs || []
      expect(roles).toHaveLength(2)
      const roleIds = toIds(roles)
      return { gs: fetched, roleIds }
    }

    it('many-to-one: creates a UC constraint with a span on role[0]', async () => {
      const { gs, roleIds } = await createBinarySchema('Author', 'Book')

      // Snapshot constraint-span count before
      const spansBefore = await payload.find({ collection: 'constraint-spans', pagination: false, depth: 0 })

      // Set the roleRelationship — the hook creates constraints
      await payload.update({
        collection: 'graph-schemas',
        id: gs.id,
        data: { roleRelationship: 'many-to-one' },
      })

      // There should be at least 1 UC constraint
      const constraints = await payload.find({
        collection: 'constraints',
        where: { kind: { equals: 'UC' } },
      })
      expect(constraints.docs.length).toBeGreaterThanOrEqual(1)

      // There should be new constraint-spans
      const spansAfter = await payload.find({ collection: 'constraint-spans', pagination: false, depth: 0 })
      const newSpanCount = spansAfter.docs.length - spansBefore.docs.length
      expect(newSpanCount).toBeGreaterThanOrEqual(1)

      // Find spans that reference one of our roles
      const roleIdSet = new Set(roleIds)
      const relevantSpans = spansAfter.docs.filter((s: any) => {
        const spanRoles = toIds(Array.isArray(s.roles) ? s.roles : [s.roles])
        return spanRoles.some((r: string) => roleIdSet.has(r))
      })
      expect(relevantSpans.length).toBeGreaterThanOrEqual(1)

      // The span should reference role[0] (many-to-one puts UC on the "many" side)
      const spanForRole0 = relevantSpans.find((s: any) => {
        const spanRoles = toIds(Array.isArray(s.roles) ? s.roles : [s.roles])
        return spanRoles.includes(roleIds[0])
      })
      expect(spanForRole0).toBeDefined()
    })

    it('one-to-many: creates a UC constraint with a span on role[1]', async () => {
      const { gs, roleIds } = await createBinarySchema('Teacher', 'Student')

      // Snapshot constraint-span count before
      const spansBefore = await payload.find({ collection: 'constraint-spans', pagination: false, depth: 0 })

      await payload.update({
        collection: 'graph-schemas',
        id: gs.id,
        data: { roleRelationship: 'one-to-many' },
      })

      const spansAfter = await payload.find({ collection: 'constraint-spans', pagination: false, depth: 0 })
      const newSpanCount = spansAfter.docs.length - spansBefore.docs.length
      expect(newSpanCount).toBeGreaterThanOrEqual(1)

      const roleIdSet = new Set(roleIds)
      const relevantSpans = spansAfter.docs.filter((s: any) => {
        const spanRoles = toIds(Array.isArray(s.roles) ? s.roles : [s.roles])
        return spanRoles.some((r: string) => roleIdSet.has(r))
      })
      expect(relevantSpans.length).toBeGreaterThanOrEqual(1)

      // The span should reference role[1] (one-to-many puts UC on role[1])
      const spanForRole1 = relevantSpans.find((s: any) => {
        const spanRoles = toIds(Array.isArray(s.roles) ? s.roles : [s.roles])
        return spanRoles.includes(roleIds[1])
      })
      expect(spanForRole1).toBeDefined()
    })

    it('one-to-one: creates 2 separate UC constraints with 1 span each', async () => {
      const { gs, roleIds } = await createBinarySchema('Passport', 'Citizen')

      // Snapshot constraint counts before
      const constraintsBefore = await payload.find({
        collection: 'constraints',
        where: { kind: { equals: 'UC' } },
        pagination: false,
      })
      const spansBefore = await payload.find({
        collection: 'constraint-spans',
        pagination: false,
      })

      await payload.update({
        collection: 'graph-schemas',
        id: gs.id,
        data: { roleRelationship: 'one-to-one' },
      })

      const constraintsAfter = await payload.find({
        collection: 'constraints',
        where: { kind: { equals: 'UC' } },
        pagination: false,
      })
      const spansAfter = await payload.find({
        collection: 'constraint-spans',
        pagination: false,
        depth: 0,
      })

      // one-to-one creates 2 NEW UC constraints (the initial one + toOneConstraint)
      const newConstraints = constraintsAfter.docs.length - constraintsBefore.docs.length
      expect(newConstraints).toBe(2)

      // one-to-one creates 2 NEW constraint-spans
      const newSpans = spansAfter.docs.length - spansBefore.docs.length
      expect(newSpans).toBe(2)

      // Each span should reference one of our roles
      const roleIdSet = new Set(roleIds)
      const ourSpans = spansAfter.docs.filter((s: any) => {
        const spanRoles = toIds(Array.isArray(s.roles) ? s.roles : [s.roles])
        return spanRoles.some((r: string) => roleIdSet.has(r))
      })
      expect(ourSpans).toHaveLength(2)

      // The two spans should reference different constraints
      const constraintIds = ourSpans.map((s: any) => typeof s.constraint === 'string' ? s.constraint : s.constraint?.id)
      expect(constraintIds[0]).not.toBe(constraintIds[1])
    })
  })

  // ---------------------------------------------------------------------------
  // Title computation
  // ---------------------------------------------------------------------------
  describe('title computation', () => {
    it('should use name as title when name is provided', async () => {
      const gs = await payload.create({
        collection: 'graph-schemas',
        data: { name: 'MySchemaName' },
      })
      expect(gs.title).toBe('MySchemaName')
    })

    it('should fall back to first reading text when name is absent', async () => {
      // Use unique nouns so the reading hook finds them
      await payload.create({
        collection: 'nouns',
        data: { name: 'Depot', objectType: 'entity' },
      })
      await payload.create({
        collection: 'nouns',
        data: { name: 'Gadget', objectType: 'entity' },
      })

      // Create schema without a name — title will initially be "undefined"
      const gs = await payload.create({
        collection: 'graph-schemas',
        data: {},
      })

      // Create reading with graphSchema set — the title hook should use the reading text
      const reading = await payload.create({
        collection: 'readings',
        data: {
          text: 'Depot stores Gadget',
          graphSchema: gs.id,
        },
      })

      // Re-fetch to pick up the updated title (title hook queries readings by graphSchema)
      // We need to trigger a title recomputation — update the schema to trigger hooks
      await payload.update({
        collection: 'graph-schemas',
        id: gs.id,
        data: {},
      })

      const fetched = await payload.findByID({
        collection: 'graph-schemas',
        id: gs.id,
      })
      expect(fetched.title).toBe('Depot stores Gadget')
    })
  })
})
