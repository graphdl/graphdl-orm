import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'

let payload: any

describe('Roles collection', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  // ---------------------------------------------------------------------------
  // Title computation
  // ---------------------------------------------------------------------------
  describe('title computation', () => {
    it('should compute title as "{noun.name} - {graphSchema.title}"', async () => {
      // Create a noun with a unique name
      const noun = await payload.create({
        collection: 'nouns',
        data: { name: 'Vehicle4', objectType: 'entity' },
      })

      // Create a graphSchema with the name set to "VehicleHasVin4"
      const graphSchema = await payload.create({
        collection: 'graph-schemas',
        data: { name: 'VehicleHasVin4' },
      })

      // graphSchema.title should be "VehicleHasVin4" (name takes priority over reading text)
      expect(graphSchema.title).toBe('VehicleHasVin4')

      // Create a role linking the noun and graphSchema
      const role = await payload.create({
        collection: 'roles',
        data: {
          noun: { relationTo: 'nouns', value: noun.id },
          graphSchema: graphSchema.id,
        },
      })

      expect(role.title).toBe('Vehicle4 - VehicleHasVin4')
    })

    it('should not contain "undefined" in the title', async () => {
      // Create a noun and graphSchema with proper names
      const noun = await payload.create({
        collection: 'nouns',
        data: { name: 'Driver4', objectType: 'entity' },
      })

      const graphSchema = await payload.create({
        collection: 'graph-schemas',
        data: { name: 'DriverHasLicense4' },
      })

      const role = await payload.create({
        collection: 'roles',
        data: {
          noun: { relationTo: 'nouns', value: noun.id },
          graphSchema: graphSchema.id,
        },
      })

      // Re-fetch to account for any afterChange repair hook
      const fetched = await payload.findByID({ collection: 'roles', id: role.id })
      expect(fetched.title).not.toContain('undefined')
    })
  })

  // ---------------------------------------------------------------------------

  // ---------------------------------------------------------------------------
  // Role ordering
  // ---------------------------------------------------------------------------
  describe('role ordering', () => {
    it('should create roles in reading text order', async () => {
      const noun1 = await payload.create({
        collection: 'nouns',
        data: { name: 'Alpha', objectType: 'entity', plural: 'alphas' },
      })
      const noun2 = await payload.create({
        collection: 'nouns',
        data: { name: 'Beta', objectType: 'value', valueType: 'string' },
      })
      const gs = await payload.create({
        collection: 'graph-schemas',
        data: { name: 'AlphaHasBeta' },
      })
      await payload.create({
        collection: 'readings',
        data: { text: 'Alpha has Beta', graphSchema: gs.id },
      })
      const roles = await payload.find({
        collection: 'roles',
        where: { graphSchema: { equals: gs.id } },
        sort: 'createdAt',
      })
      expect(roles.docs.length).toBe(2)
      const firstNounId = typeof roles.docs[0].noun?.value === 'string' ? roles.docs[0].noun.value : (roles.docs[0].noun?.value as any)?.id
      const secondNounId = typeof roles.docs[1].noun?.value === 'string' ? roles.docs[1].noun.value : (roles.docs[1].noun?.value as any)?.id
      expect(firstNounId).toBe(noun1.id)
      expect(secondNounId).toBe(noun2.id)
    })
  })

  // ---------------------------------------------------------------------------
})
