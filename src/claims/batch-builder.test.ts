import { describe, it, expect } from 'vitest'
import { BatchBuilder } from './batch-builder'

describe('BatchBuilder', () => {
  it('accumulates entities', () => {
    const builder = new BatchBuilder('tickets')
    builder.addEntity('Noun', { name: 'Customer', objectType: 'entity' })
    builder.addEntity('Reading', { text: 'Customer has Name', graphSchemaId: 'gs1' })
    const batch = builder.toBatch()
    expect(batch.entities).toHaveLength(2)
    expect(batch.domain).toBe('tickets')
    expect(batch.entities[0].type).toBe('Noun')
    expect(batch.entities[1].type).toBe('Reading')
  })

  it('generates UUIDs for entities without ids', () => {
    const builder = new BatchBuilder('tickets')
    builder.addEntity('Noun', { name: 'Customer' })
    const batch = builder.toBatch()
    expect(batch.entities[0].id).toBeDefined()
    expect(batch.entities[0].id.length).toBeGreaterThan(0)
  })

  it('preserves provided ids', () => {
    const builder = new BatchBuilder('tickets')
    builder.addEntity('Noun', { name: 'Customer' }, 'my-custom-id')
    const batch = builder.toBatch()
    expect(batch.entities[0].id).toBe('my-custom-id')
  })

  it('supports find-or-add by type and key', () => {
    const builder = new BatchBuilder('tickets')
    const id1 = builder.ensureEntity('Noun', 'name', 'Customer', { name: 'Customer', objectType: 'entity' })
    const id2 = builder.ensureEntity('Noun', 'name', 'Customer', { name: 'Customer', objectType: 'entity' })
    expect(id1).toBe(id2) // same entity, not duplicated
    expect(builder.toBatch().entities).toHaveLength(1)
  })

  it('tracks entity count', () => {
    const builder = new BatchBuilder('tickets')
    builder.addEntity('Noun', { name: 'A' })
    builder.addEntity('Noun', { name: 'B' })
    builder.addEntity('Reading', { text: 'A has B' })
    expect(builder.entityCount).toBe(3)
  })
})
