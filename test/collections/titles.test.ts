import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'

let payload: any

describe('Computed title generation', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  it('Role title', async () => {
    // Create noun "Car6" and schema "CarHasVin6"
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'Car6', objectType: 'entity' },
    })

    const schema = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'CarHasVin6' },
    })

    // Create a role referencing the noun and schema
    const role = await payload.create({
      collection: 'roles',
      data: {
        noun: { relationTo: 'nouns', value: noun.id },
        graphSchema: schema.id,
      },
    })

    // The Role title hook computes: `${noun.name} - ${graphSchema.title}`
    // GraphSchema title = "CarHasVin6" (uses name)
    expect(role.title).toBe('Car6 - CarHasVin6')
  })

  it('ConstraintSpan title', async () => {
    // Create noun, schema, role
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'Widget6', objectType: 'entity' },
    })

    const schema = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'WidgetSchema6' },
    })

    const role = await payload.create({
      collection: 'roles',
      data: {
        noun: { relationTo: 'nouns', value: noun.id },
        graphSchema: schema.id,
      },
    })

    // Create a UC Alethic constraint
    const constraint = await payload.create({
      collection: 'constraints',
      data: {
        kind: 'UC',
        modality: 'Alethic',
      },
    })

    // Create a constraint-span linking the constraint to the role
    const constraintSpan = await payload.create({
      collection: 'constraint-spans',
      data: {
        constraint: constraint.id,
        roles: [role.id],
      },
    })

    // The ConstraintSpan title hook computes:
    // `${constraint.modality} ${constraint.kind} - ${roleNames} - ${schemaTitle}`
    // So it should contain "Alethic" and "UC"
    expect(constraintSpan.title).toContain('Alethic')
    expect(constraintSpan.title).toContain('UC')
  })

  it('GraphSchema title uses name', async () => {
    const schema = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'MyUniqueSchemaTitle6' },
    })

    // The GraphSchema title hook returns: `${data.name || originalDoc.name || primaryReading.text}`
    expect(schema.title).toBe('MyUniqueSchemaTitle6')
  })
})
