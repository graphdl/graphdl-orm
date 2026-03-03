import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'

let payload: any
let mermaidOutput: any

describe('Mermaid diagram generator', () => {
  beforeAll(async () => {
    payload = await initPayload()
    await payload.db.connection.dropDatabase()

    // Create entity and value nouns
    const customer = await payload.create({
      collection: 'nouns',
      data: { name: 'Customer', objectType: 'entity', plural: 'customers', permissions: ['create', 'read'] },
    })
    const email = await payload.create({
      collection: 'nouns',
      data: { name: 'EmailAddress', objectType: 'value', valueType: 'string' },
    })
    await payload.update({ collection: 'nouns', id: customer.id, data: { referenceScheme: [email.id] } })

    const order = await payload.create({
      collection: 'nouns',
      data: { name: 'Order', objectType: 'entity', plural: 'orders', permissions: ['create', 'read'] },
    })
    const orderId = await payload.create({
      collection: 'nouns',
      data: { name: 'OrderId', objectType: 'value', valueType: 'string' },
    })
    await payload.update({ collection: 'nouns', id: order.id, data: { referenceScheme: [orderId.id] } })

    // Create a reading: Customer submits Order (1:*)
    const gs1 = await payload.create({ collection: 'graph-schemas', data: { name: 'CustomerSubmitsOrder' } })
    await payload.create({ collection: 'readings', data: { text: 'Customer submits Order', graphSchema: gs1.id } })
    await payload.update({ collection: 'graph-schemas', id: gs1.id, data: { roleRelationship: 'one-to-many' } })

    // Create a state machine for Order
    const smDef = await payload.create({
      collection: 'state-machine-definitions',
      data: { noun: { relationTo: 'nouns', value: order.id } },
    })
    const pending = await payload.create({ collection: 'statuses', data: { name: 'Pending', stateMachineDefinition: smDef.id } })
    const shipped = await payload.create({ collection: 'statuses', data: { name: 'Shipped', stateMachineDefinition: smDef.id } })
    const delivered = await payload.create({ collection: 'statuses', data: { name: 'Delivered', stateMachineDefinition: smDef.id } })

    const shipEvent = await payload.create({ collection: 'event-types', data: { name: 'ship' } })
    const deliverEvent = await payload.create({ collection: 'event-types', data: { name: 'deliver' } })

    await payload.create({ collection: 'transitions', data: { from: pending.id, to: shipped.id, eventType: shipEvent.id } })
    await payload.create({ collection: 'transitions', data: { from: shipped.id, to: delivered.id, eventType: deliverEvent.id } })

    // First create an OpenAPI generator (needed for UML)
    await payload.create({
      collection: 'generators',
      data: { title: 'Test API', version: '1.0.0', databaseEngine: 'Payload' },
    })

    // Now create the mermaid generator
    const gen = await payload.create({
      collection: 'generators',
      data: { title: 'Test Diagrams', version: '1.0.0', databaseEngine: 'Payload', outputFormat: 'mermaid' },
    })
    mermaidOutput = gen.output
  }, 120_000)

  it('should generate an ORM2 ER diagram', () => {
    const orm2 = mermaidOutput?.files?.['diagrams/orm2.mmd']
    expect(orm2).toBeDefined()
    expect(orm2).toContain('erDiagram')
    expect(orm2).toContain('Customer')
    expect(orm2).toContain('Order')
    expect(orm2).toContain('Customer submits Order')
  })

  it('should include entity reference schemes as PK', () => {
    const orm2 = mermaidOutput?.files?.['diagrams/orm2.mmd']
    expect(orm2).toContain('string EmailAddress PK')
  })

  it('should generate a state machine diagram', () => {
    const smFile = mermaidOutput?.files?.['diagrams/state-order.mmd']
    expect(smFile).toBeDefined()
    expect(smFile).toContain('stateDiagram-v2')
    expect(smFile).toContain('[*] --> Pending')
    expect(smFile).toContain('Pending --> Shipped: ship')
    expect(smFile).toContain('Shipped --> Delivered: deliver')
    expect(smFile).toContain('Delivered --> [*]')
  })

  it('should generate a UML class diagram from OpenAPI output', () => {
    const uml = mermaidOutput?.files?.['diagrams/uml-classes.mmd']
    expect(uml).toBeDefined()
    expect(uml).toContain('classDiagram')
    expect(uml).toContain('class Customer')
    expect(uml).toContain('class Order')
  })
})
