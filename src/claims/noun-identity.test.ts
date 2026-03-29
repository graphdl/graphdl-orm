import { describe, it, expect } from 'vitest'
import { BatchBuilder } from './batch-builder'
import { ensureNoun } from './steps'

describe('noun identity', () => {
  it('uses noun name as entity ID (CSDP: identity IS the name)', () => {
    const b = new BatchBuilder('domain-1')
    const id = ensureNoun(b, 'Customer', { objectType: 'entity' }, 'domain-1')
    expect(id).toBe('Customer')
    const batch = b.toBatch()
    expect(batch.entities[0].id).toBe('Customer')
    expect(batch.entities[0].data.name).toBe('Customer')
  })

  it('deduplicates by name', () => {
    const b = new BatchBuilder('domain-1')
    const id1 = ensureNoun(b, 'Customer', { objectType: 'entity' }, 'domain-1')
    const id2 = ensureNoun(b, 'Customer', { objectType: 'entity' }, 'domain-1')
    expect(id1).toBe(id2)
    expect(b.toBatch().entities.filter(e => e.type === 'Noun')).toHaveLength(1)
  })
})
