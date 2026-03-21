import { describe, it, expect } from 'vitest'
import {
  buildPopulation,
  fanOutCollect,
  executeQuery,
  type EntityStub,
  type QueryRequest,
} from './query'

/**
 * Creates a mock EntityStub that returns canned data.
 */
function mockStub(data: { id: string; type: string; data: Record<string, unknown> } | null): EntityStub {
  return {
    get: async () => data,
  }
}

describe('worker/query', () => {
  describe('buildPopulation', () => {
    it('converts entities to Population format with correct bindings', () => {
      const entities = [
        { id: 'sr-001', data: { Status: 'Investigating', Priority: 'High' } },
        { id: 'sr-002', data: { Status: 'Resolved', Priority: 'Low' } },
      ]

      const population = buildPopulation(
        'SupportRequest',
        entities,
        'ft-status',
        ['Status'],
      )

      expect(population.facts['ft-status']).toBeDefined()
      expect(population.facts['ft-status']).toHaveLength(2)

      const first = population.facts['ft-status'][0]
      expect(first.fact_type_id).toBe('ft-status')
      // Should have bindings for nounType (the entity ref) and each value field
      expect(first.bindings).toContainEqual(['SupportRequest', 'sr-001'])
      expect(first.bindings).toContainEqual(['Status', 'Investigating'])

      const second = population.facts['ft-status'][1]
      expect(second.bindings).toContainEqual(['SupportRequest', 'sr-002'])
      expect(second.bindings).toContainEqual(['Status', 'Resolved'])
    })

    it('includes multiple value fields in bindings', () => {
      const entities = [
        { id: 'p-1', data: { City: 'Austin', Country: 'US' } },
      ]

      const population = buildPopulation(
        'Person',
        entities,
        'ft-location',
        ['City', 'Country'],
      )

      const fact = population.facts['ft-location'][0]
      expect(fact.bindings).toContainEqual(['Person', 'p-1'])
      expect(fact.bindings).toContainEqual(['City', 'Austin'])
      expect(fact.bindings).toContainEqual(['Country', 'US'])
    })

    it('returns empty facts when entities array is empty', () => {
      const population = buildPopulation('X', [], 'ft-x', ['field'])
      expect(population.facts['ft-x']).toEqual([])
    })
  })

  describe('fanOutCollect', () => {
    it('collects data from multiple entity stubs in parallel', async () => {
      const stubs: Record<string, EntityStub> = {
        'e-1': mockStub({ id: 'e-1', type: 'User', data: { name: 'Alice' } }),
        'e-2': mockStub({ id: 'e-2', type: 'User', data: { name: 'Bob' } }),
        'e-3': mockStub({ id: 'e-3', type: 'User', data: { name: 'Carol' } }),
      }

      const results = await fanOutCollect(
        ['e-1', 'e-2', 'e-3'],
        (id) => stubs[id],
      )

      expect(results).toHaveLength(3)
      expect(results.map(r => r.id).sort()).toEqual(['e-1', 'e-2', 'e-3'])
      expect(results.find(r => r.id === 'e-1')!.data).toEqual({ name: 'Alice' })
    })

    it('handles null entities (deleted DOs) gracefully', async () => {
      const stubs: Record<string, EntityStub> = {
        'e-1': mockStub({ id: 'e-1', type: 'User', data: { name: 'Alice' } }),
        'e-2': mockStub(null), // deleted / empty DO
        'e-3': mockStub({ id: 'e-3', type: 'User', data: { name: 'Carol' } }),
      }

      const results = await fanOutCollect(
        ['e-1', 'e-2', 'e-3'],
        (id) => stubs[id],
      )

      // null entities should be filtered out
      expect(results).toHaveLength(2)
      expect(results.map(r => r.id).sort()).toEqual(['e-1', 'e-3'])
    })

    it('batches requests (batch of 2 for 5 entities = 3 batches)', async () => {
      let concurrentCalls = 0
      let maxConcurrent = 0

      const makeStub = (id: string): EntityStub => ({
        get: async () => {
          concurrentCalls++
          maxConcurrent = Math.max(maxConcurrent, concurrentCalls)
          // Simulate async work
          await new Promise(resolve => setTimeout(resolve, 10))
          concurrentCalls--
          return { id, type: 'Item', data: { value: id } }
        },
      })

      const ids = ['a', 'b', 'c', 'd', 'e']
      const stubs: Record<string, EntityStub> = {}
      for (const id of ids) stubs[id] = makeStub(id)

      const results = await fanOutCollect(
        ids,
        (id) => stubs[id],
        2, // batch size
      )

      expect(results).toHaveLength(5)
      // With batch size 2, max concurrent should be at most 2
      expect(maxConcurrent).toBeLessThanOrEqual(2)
    })

    it('handles empty entity list', async () => {
      const results = await fanOutCollect([], (id) => mockStub(null))
      expect(results).toEqual([])
    })
  })

  describe('executeQuery', () => {
    it('end-to-end: creates stubs, fans out, filters with predicate', async () => {
      const stubs: Record<string, EntityStub> = {
        'sr-001': mockStub({ id: 'sr-001', type: 'SupportRequest', data: { Status: 'Investigating', Priority: 'High' } }),
        'sr-002': mockStub({ id: 'sr-002', type: 'SupportRequest', data: { Status: 'Resolved', Priority: 'Low' } }),
        'sr-003': mockStub({ id: 'sr-003', type: 'SupportRequest', data: { Status: 'Investigating', Priority: 'Medium' } }),
      }

      const query: QueryRequest = {
        nounType: 'SupportRequest',
        factTypeId: 'ft-status',
        filterBindings: [['Status', 'Investigating']],
      }

      const result = await executeQuery(
        ['sr-001', 'sr-002', 'sr-003'],
        (id) => stubs[id],
        query,
        ['Status', 'Priority'],
      )

      expect(result.count).toBe(2)
      expect(result.matches.sort()).toEqual(['sr-001', 'sr-003'])
    })

    it('returns empty when no entities match the filter', async () => {
      const stubs: Record<string, EntityStub> = {
        'p-1': mockStub({ id: 'p-1', type: 'Person', data: { City: 'Denver' } }),
        'p-2': mockStub({ id: 'p-2', type: 'Person', data: { City: 'Denver' } }),
      }

      const query: QueryRequest = {
        nounType: 'Person',
        factTypeId: 'ft-city',
        filterBindings: [['City', 'Austin']],
      }

      const result = await executeQuery(
        ['p-1', 'p-2'],
        (id) => stubs[id],
        query,
        ['City'],
      )

      expect(result.count).toBe(0)
      expect(result.matches).toEqual([])
    })

    it('returns all entities when filter bindings are empty', async () => {
      const stubs: Record<string, EntityStub> = {
        'x-1': mockStub({ id: 'x-1', type: 'X', data: { val: 'a' } }),
        'x-2': mockStub({ id: 'x-2', type: 'X', data: { val: 'b' } }),
      }

      const query: QueryRequest = {
        nounType: 'X',
        factTypeId: 'ft-val',
        filterBindings: [],
      }

      const result = await executeQuery(
        ['x-1', 'x-2'],
        (id) => stubs[id],
        query,
        ['val'],
      )

      expect(result.count).toBe(2)
      expect(result.matches.sort()).toEqual(['x-1', 'x-2'])
    })

    it('handles deleted entities during fan-out', async () => {
      const stubs: Record<string, EntityStub> = {
        'a': mockStub({ id: 'a', type: 'T', data: { Color: 'red' } }),
        'b': mockStub(null), // deleted
        'c': mockStub({ id: 'c', type: 'T', data: { Color: 'red' } }),
      }

      const query: QueryRequest = {
        nounType: 'T',
        factTypeId: 'ft-color',
        filterBindings: [['Color', 'red']],
      }

      const result = await executeQuery(
        ['a', 'b', 'c'],
        (id) => stubs[id],
        query,
        ['Color'],
      )

      expect(result.count).toBe(2)
      expect(result.matches.sort()).toEqual(['a', 'c'])
    })
  })
})
