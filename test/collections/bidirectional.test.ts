import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'

let payload: any

/** Extract plain string IDs from a field that may contain populated objects. */
function toIds(arr: any[]): string[] {
  if (!arr) return []
  return arr.map((r: any) => (typeof r === 'string' ? r : r?.id || r))
}

describe('Bidirectional sync (join fields)', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  it('reading with graphSchema should appear in schema.readings join field', async () => {
    // Create a graph schema
    const schema = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'BiDirSchema7' },
    })

    // Create a reading that points to this schema via the relationship field
    const reading = await payload.create({
      collection: 'readings',
      data: { text: 'BiDirReading7 does something', graphSchema: schema.id },
    })

    // Fetch the schema — readings is a join field returning { docs: [...] }
    const fetched = await payload.findByID({
      collection: 'graph-schemas',
      id: schema.id,
      depth: 1,
    })

    const readingIds = toIds(fetched.readings?.docs || [])
    expect(readingIds).toContain(reading.id)
  })

  it('role with graphSchema should appear in schema.roles join field', async () => {
    // Create a noun for the role
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'BiDirNoun8', objectType: 'entity' },
    })

    // Create a graph schema
    const schema = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'BiDirSchemaRoles8' },
    })

    // Create a role referencing the graph schema
    const role = await payload.create({
      collection: 'roles',
      data: {
        noun: { relationTo: 'nouns', value: noun.id },
        graphSchema: schema.id,
      },
    })

    // Fetch the schema — roles is a join field returning { docs: [...] }
    const fetched = await payload.findByID({
      collection: 'graph-schemas',
      id: schema.id,
      depth: 0,
    })

    const roleIds = toIds(fetched.roles?.docs || [])
    expect(roleIds).toContain(role.id)

    // The role's graphSchema should be set
    const fetchedRole = await payload.findByID({
      collection: 'roles',
      id: role.id,
      depth: 0,
    })
    expect(fetchedRole.graphSchema).toBe(schema.id)
  })

  it('status with stateMachineDefinition should appear in definition.statuses join field', async () => {
    // Create a noun for the state machine definition
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'BiDirNounSM9', objectType: 'entity' },
    })

    // Create a state-machine-definition with noun (polymorphic relationship)
    const definition = await payload.create({
      collection: 'state-machine-definitions',
      data: { noun: { relationTo: 'nouns', value: noun.id } },
    })

    // Create a status referencing the definition
    const status = await payload.create({
      collection: 'statuses',
      data: {
        name: 'Active9',
        stateMachineDefinition: definition.id,
      },
    })

    // Fetch the definition — statuses is a join field returning { docs: [...] }
    const fetched = await payload.findByID({
      collection: 'state-machine-definitions',
      id: definition.id,
      depth: 0,
    })

    const statusIds = toIds(fetched.statuses?.docs || [])
    expect(statusIds).toContain(status.id)
  })
})
