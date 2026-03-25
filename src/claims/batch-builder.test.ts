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

  it('updates an existing entity by id', () => {
    const builder = new BatchBuilder('tickets')
    const id = builder.addEntity('Noun', { name: 'Customer', objectType: 'entity' })

    const updated = builder.updateEntity(id, { superType: 'parent-id' })

    expect(updated).toBe(true)
    const entity = builder.findEntity(id)
    expect(entity!.data.superType).toBe('parent-id')
    expect(entity!.data.name).toBe('Customer') // original fields preserved
  })

  it('returns false when updating non-existent entity', () => {
    const builder = new BatchBuilder('tickets')
    const updated = builder.updateEntity('non-existent-id', { foo: 'bar' })
    expect(updated).toBe(false)
  })

  it('finds an entity by id', () => {
    const builder = new BatchBuilder('tickets')
    const id = builder.addEntity('Noun', { name: 'Customer' })
    const entity = builder.findEntity(id)
    expect(entity).toBeDefined()
    expect(entity!.data.name).toBe('Customer')
  })

  it('returns undefined for non-existent entity', () => {
    const builder = new BatchBuilder('tickets')
    const entity = builder.findEntity('non-existent-id')
    expect(entity).toBeUndefined()
  })

  it('finds entities by type and filter', () => {
    const builder = new BatchBuilder('tickets')
    builder.addEntity('Noun', { name: 'Customer', domain: 'd1' })
    builder.addEntity('Noun', { name: 'Name', domain: 'd1' })
    builder.addEntity('Reading', { text: 'Customer has Name', domain: 'd1' })
    builder.addEntity('Noun', { name: 'Order', domain: 'd2' })

    const allNouns = builder.findEntities('Noun')
    expect(allNouns).toHaveLength(3)

    const d1Nouns = builder.findEntities('Noun', { domain: 'd1' })
    expect(d1Nouns).toHaveLength(2)

    const customerNouns = builder.findEntities('Noun', { name: 'Customer' })
    expect(customerNouns).toHaveLength(1)
    expect(customerNouns[0].data.name).toBe('Customer')
  })

  it('returns empty array when no entities match filter', () => {
    const builder = new BatchBuilder('tickets')
    builder.addEntity('Noun', { name: 'Customer' })
    const result = builder.findEntities('Reading')
    expect(result).toHaveLength(0)
  })
})
