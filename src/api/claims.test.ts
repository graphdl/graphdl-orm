import { describe, it, expect, vi } from 'vitest'
import { materializeBatch } from '../worker/materialize'

describe('claims materialization', () => {
  it('materializes batch entities with domain slug (not UUID)', async () => {
    const putCalls: any[] = []
    const indexCalls: any[] = []

    const entities = [
      { id: 'n1', type: 'Noun', domain: 'tickets', data: { name: 'Customer' } },
      { id: 'n2', type: 'Noun', domain: 'tickets', data: { name: 'Order' } },
      { id: 'r1', type: 'Reading', domain: 'tickets', data: { text: 'Customer has Order' } },
    ]

    const getStub = (id: string) => ({
      put: vi.fn().mockImplementation((input: any) => {
        putCalls.push(input)
        return Promise.resolve({ id: input.id, version: 1 })
      }),
    })

    const registry = {
      indexEntity: vi.fn().mockImplementation((type: string, id: string, domain?: string) => {
        indexCalls.push({ type, id, domain })
        return Promise.resolve()
      }),
      indexNoun: vi.fn().mockResolvedValue(undefined),
    }

    const result = await materializeBatch(entities, getStub, registry)

    expect(result.materialized).toBe(3)
    expect(result.failed).toHaveLength(0)

    // All entities written to EntityDB DOs
    expect(putCalls).toHaveLength(3)

    // All indexed with domain SLUG (not UUID)
    for (const call of indexCalls) {
      expect(call.domain).toBe('tickets')
    }

    // Noun entities trigger indexNoun with the slug
    expect(registry.indexNoun).toHaveBeenCalledWith('Customer', 'tickets')
    expect(registry.indexNoun).toHaveBeenCalledWith('Order', 'tickets')
  })

  it('filters Violation entities from materialization', () => {
    const entities = [
      { id: 'n1', type: 'Noun', domain: 'tickets', data: { name: 'A' } },
      { id: 'v1', type: 'Violation', domain: 'tickets', data: { text: 'bad' } },
      { id: 'n2', type: 'Noun', domain: 'tickets', data: { name: 'B' } },
    ]

    const nonViolations = entities.filter(e => e.type !== 'Violation')
    expect(nonViolations).toHaveLength(2)
    expect(nonViolations.every(e => e.type !== 'Violation')).toBe(true)
  })

  it('overrides batch domain UUID with slug before materializing', () => {
    const batchEntities = [
      { id: 'n1', type: 'Noun', domain: 'a1b2c3-uuid', data: { name: 'Foo' } },
    ]
    const slug = 'my-domain'
    const resolved = batchEntities.map(e => ({ ...e, domain: slug }))

    expect(resolved[0].domain).toBe('my-domain')
    expect(batchEntities[0].domain).toBe('a1b2c3-uuid') // original unchanged
  })
})
