import { describe, it, expect, vi } from 'vitest'
import { EntityDataLoader } from './entity-data-loader'
import type { RegistryStub, EntityStub } from './entity-data-loader'

describe('EntityDataLoader', () => {
  it('loads nouns by fan-out to entity stubs', async () => {
    const entities = [
      { id: 'n1', type: 'Noun', data: { name: 'Customer', object_type: 'entity', domain_id: 'd1' } },
      { id: 'n2', type: 'Noun', data: { name: 'Order', object_type: 'entity', domain_id: 'd1' } },
    ]
    const stubs = new Map<string, EntityStub>(
      entities.map(e => [e.id, { get: vi.fn().mockResolvedValue(e) }]),
    )
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue(['n1', 'n2']) }
    const loader = new EntityDataLoader(registry, (id) => stubs.get(id)!)
    const nouns = await loader.queryNouns('d1')
    expect(nouns).toHaveLength(2)
    expect(nouns[0].name).toBe('Customer')
    expect(nouns[1].name).toBe('Order')
  })

  it('returns empty array when no entities found', async () => {
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue([]) }
    const loader = new EntityDataLoader(registry, () => ({ get: vi.fn().mockResolvedValue(null) }))
    const nouns = await loader.queryNouns('d1')
    expect(nouns).toHaveLength(0)
  })

  it('filters null responses from unreachable DOs', async () => {
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue(['n1', 'n2']) }
    const stubs = new Map<string, EntityStub>([
      ['n1', { get: vi.fn().mockResolvedValue({ id: 'n1', type: 'Noun', data: { name: 'A', object_type: 'entity', domain_id: 'd1' } }) }],
      ['n2', { get: vi.fn().mockResolvedValue(null) }],
    ])
    const loader = new EntityDataLoader(registry, (id) => stubs.get(id)!)
    const nouns = await loader.queryNouns('d1')
    expect(nouns).toHaveLength(1)
    expect(nouns[0].name).toBe('A')
  })

  it('passes entity type and domain to registry', async () => {
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue([]) }
    const loader = new EntityDataLoader(registry, () => ({ get: vi.fn().mockResolvedValue(null) }))
    await loader.queryNouns('my-domain')
    expect(registry.getEntityIds).toHaveBeenCalledWith('Noun', 'my-domain')
  })

  it('batches fan-out at 50 concurrent requests', async () => {
    // Create 75 entity IDs to verify batching (50 + 25)
    const ids = Array.from({ length: 75 }, (_, i) => `n${i}`)
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue(ids) }

    let maxConcurrent = 0
    let currentConcurrent = 0

    const loader = new EntityDataLoader(registry, (id) => ({
      get: vi.fn().mockImplementation(async () => {
        currentConcurrent++
        if (currentConcurrent > maxConcurrent) maxConcurrent = currentConcurrent
        // Simulate async delay
        await new Promise(r => setTimeout(r, 1))
        currentConcurrent--
        return { id, type: 'Noun', data: { name: `Noun_${id}`, object_type: 'entity', domain_id: 'd1' } }
      }),
    }))

    const nouns = await loader.queryNouns('d1')
    expect(nouns).toHaveLength(75)
    expect(maxConcurrent).toBeLessThanOrEqual(50)
  })

  it('maps entity data fields to Row-compatible format', async () => {
    const entity = {
      id: 'n1',
      type: 'Noun',
      data: {
        name: 'Priority',
        object_type: 'value',
        domain_id: 'd1',
        value_type: 'string',
        enum_values: '["low","high"]',
        super_type_id: null,
        super_type_name: 'BaseValue',
        plural: 'Priorities',
        prompt_text: 'The priority level',
      },
    }
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue(['n1']) }
    const stubs = new Map<string, EntityStub>([['n1', { get: vi.fn().mockResolvedValue(entity) }]])
    const loader = new EntityDataLoader(registry, (id) => stubs.get(id)!)
    const nouns = await loader.queryNouns('d1')
    expect(nouns).toHaveLength(1)
    const row = nouns[0]
    expect(row.id).toBe('n1')
    expect(row.name).toBe('Priority')
    expect(row.object_type).toBe('value')
    expect(row.domain_id).toBe('d1')
    expect(row.value_type).toBe('string')
    expect(row.enum_values).toBe('["low","high"]')
    expect(row.super_type_name).toBe('BaseValue')
    expect(row.plural).toBe('Priorities')
    expect(row.prompt_text).toBe('The priority level')
  })

  it('returns stub methods for unimplemented query types', () => {
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue([]) }
    const loader = new EntityDataLoader(registry, () => ({ get: vi.fn().mockResolvedValue(null) }))
    // These stub methods exist and return empty arrays (sync, to match DataLoader interface for now)
    expect(loader.queryGraphSchemas('d1')).toEqual([])
    expect(loader.queryReadings('d1')).toEqual([])
    expect(loader.queryRoles()).toEqual([])
    expect(loader.queryConstraints('d1')).toEqual([])
    expect(loader.queryConstraintSpans()).toEqual([])
    expect(loader.queryStateMachineDefs('d1')).toEqual([])
    expect(loader.queryStatuses('d1')).toEqual([])
    expect(loader.queryTransitions('d1')).toEqual([])
    expect(loader.queryEventTypes('d1')).toEqual([])
    expect(loader.queryGuards('d1')).toEqual([])
    expect(loader.queryVerbs('d1')).toEqual([])
    expect(loader.queryFunctions('d1')).toEqual([])
  })
})
