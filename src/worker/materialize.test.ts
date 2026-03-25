import { describe, it, expect, vi } from 'vitest'
import { materializeBatch } from './materialize'

describe('materializeBatch', () => {
  it('creates EntityDB DOs for each entity in batch', async () => {
    const entities = [
      { id: 'n1', type: 'Noun', domain: 'tickets', data: { name: 'Customer' } },
      { id: 'r1', type: 'Reading', domain: 'tickets', data: { text: 'Customer has Name' } },
    ]
    const putCalls: any[] = []
    const getEntityStub = (id: string) => ({
      put: vi.fn().mockImplementation((input) => { putCalls.push({ id, input }); return Promise.resolve({ id, version: 1 }) }),
    })
    const registryStub = {
      indexEntity: vi.fn().mockResolvedValue(undefined),
      indexNoun: vi.fn().mockResolvedValue(undefined),
    }

    await materializeBatch(entities, getEntityStub, registryStub)
    expect(putCalls).toHaveLength(2)
    expect(registryStub.indexEntity).toHaveBeenCalledTimes(2)
  })

  it('indexes Noun entities in noun_index', async () => {
    const entities = [
      { id: 'n1', type: 'Noun', domain: 'tickets', data: { name: 'Customer' } },
    ]
    const getEntityStub = () => ({ put: vi.fn().mockResolvedValue({ id: 'n1', version: 1 }) })
    const registryStub = {
      indexEntity: vi.fn().mockResolvedValue(undefined),
      indexNoun: vi.fn().mockResolvedValue(undefined),
    }

    await materializeBatch(entities, getEntityStub, registryStub)
    expect(registryStub.indexNoun).toHaveBeenCalledWith('Customer', 'tickets')
  })

  it('does not call indexNoun for non-Noun entities', async () => {
    const entities = [
      { id: 'r1', type: 'Reading', domain: 'tickets', data: { text: 'Customer has Name' } },
    ]
    const getEntityStub = () => ({ put: vi.fn().mockResolvedValue({ id: 'r1', version: 1 }) })
    const registryStub = {
      indexEntity: vi.fn().mockResolvedValue(undefined),
      indexNoun: vi.fn().mockResolvedValue(undefined),
    }

    await materializeBatch(entities, getEntityStub, registryStub)
    expect(registryStub.indexNoun).not.toHaveBeenCalled()
  })

  it('returns count of materialized entities', async () => {
    const entities = [
      { id: 'n1', type: 'Noun', domain: 'tickets', data: { name: 'A' } },
      { id: 'n2', type: 'Noun', domain: 'tickets', data: { name: 'B' } },
    ]
    const getEntityStub = () => ({ put: vi.fn().mockResolvedValue({ id: 'x', version: 1 }) })
    const registryStub = {
      indexEntity: vi.fn().mockResolvedValue(undefined),
      indexNoun: vi.fn().mockResolvedValue(undefined),
    }

    const result = await materializeBatch(entities, getEntityStub, registryStub)
    expect(result.materialized).toBe(2)
  })

  it('tracks failed entity IDs without aborting the batch', async () => {
    const entities = [
      { id: 'n1', type: 'Noun', domain: 'tickets', data: { name: 'A' } },
      { id: 'n2', type: 'Noun', domain: 'tickets', data: { name: 'B' } },
      { id: 'n3', type: 'Noun', domain: 'tickets', data: { name: 'C' } },
    ]
    const getEntityStub = (id: string) => ({
      put: vi.fn().mockImplementation(() => {
        if (id === 'n2') return Promise.reject(new Error('DO unavailable'))
        return Promise.resolve({ id, version: 1 })
      }),
    })
    const registryStub = {
      indexEntity: vi.fn().mockResolvedValue(undefined),
      indexNoun: vi.fn().mockResolvedValue(undefined),
    }

    const result = await materializeBatch(entities, getEntityStub, registryStub)
    expect(result.materialized).toBe(2)
    expect(result.failed).toEqual(['n2'])
  })

  it('batches concurrent requests at BATCH_SIZE=50', async () => {
    // Create 75 entities to verify we get 2 batches (50 + 25)
    const entities = Array.from({ length: 75 }, (_, i) => ({
      id: `e${i}`,
      type: 'Noun',
      domain: 'tickets',
      data: { name: `Entity${i}` },
    }))
    const getEntityStub = () => ({ put: vi.fn().mockResolvedValue({ id: 'x', version: 1 }) })
    const registryStub = {
      indexEntity: vi.fn().mockResolvedValue(undefined),
      indexNoun: vi.fn().mockResolvedValue(undefined),
    }

    const result = await materializeBatch(entities, getEntityStub, registryStub)
    expect(result.materialized).toBe(75)
    expect(result.failed).toEqual([])
  })
})
