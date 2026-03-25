import { describe, it, expect, vi, beforeEach } from 'vitest'
import { createViolation, createFailure, persistViolations } from './outcomes'
import type { Env } from '../types'

/**
 * Creates a mock Env with EntityDB and RegistryDB stubs.
 * Tracks all put/indexEntity calls for assertion.
 */
function createMockEnv() {
  const putCalls: Array<{ name: string; data: any }> = []
  const indexCalls: Array<{ nounType: string; entityId: string; domain: string }> = []

  const mockStub = (name: string) => ({
    put: vi.fn(async (data: any) => {
      putCalls.push({ name, data })
      return { id: data.id, version: 1 }
    }),
  })

  const mockRegistry = {
    indexEntity: vi.fn(async (nounType: string, entityId: string, domain: string) => {
      indexCalls.push({ nounType, entityId, domain })
    }),
  }

  const env = {
    ENTITY_DB: {
      idFromName: vi.fn((name: string) => name),
      get: vi.fn((name: string) => mockStub(name)),
    },
    REGISTRY_DB: {
      idFromName: vi.fn(() => 'global'),
      get: vi.fn(() => mockRegistry),
    },
    DOMAIN_DB: {
      idFromName: vi.fn(() => 'domain-id'),
      get: vi.fn(),
    },
    ENVIRONMENT: 'test',
  } as unknown as Env

  return { env, putCalls, indexCalls, mockRegistry }
}

describe('outcomes', () => {
  beforeEach(() => {
    vi.stubGlobal('crypto', { randomUUID: vi.fn(() => 'test-uuid-1234') })
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  describe('createViolation', () => {
    it('creates an EntityDB DO and indexes it in the Registry', async () => {
      const { env, putCalls, indexCalls } = createMockEnv()

      const id = await createViolation(env, {
        domain: 'support',
        constraintId: 'uc-status',
        text: 'Status value "Unknown" is not in the allowed set.',
      })

      expect(id).toBe('test-uuid-1234')

      // EntityDB stub.put was called with correct entity shape
      expect(putCalls).toHaveLength(1)
      expect(putCalls[0].data.id).toBe('test-uuid-1234')
      expect(putCalls[0].data.type).toBe('Violation')
      expect(putCalls[0].data.data.domain).toBe('support')
      expect(putCalls[0].data.data.constraintId).toBe('uc-status')
      expect(putCalls[0].data.data.text).toBe('Status value "Unknown" is not in the allowed set.')
      expect(putCalls[0].data.data.severity).toBe('error')
      expect(putCalls[0].data.data.occurredAt).toBeDefined()
      expect(putCalls[0].data.data.resourceId).toBeNull()
      expect(putCalls[0].data.data.batchId).toBeNull()

      // Registry indexEntity was called
      expect(indexCalls).toHaveLength(1)
      expect(indexCalls[0].nounType).toBe('Violation')
      expect(indexCalls[0].entityId).toBe('test-uuid-1234')
      expect(indexCalls[0].domain).toBe('support')
    })

    it('sets optional resourceId and batchId when provided', async () => {
      const { env, putCalls } = createMockEnv()

      await createViolation(env, {
        domain: 'hr',
        constraintId: 'mc-salary',
        text: 'Employee must have a Salary.',
        resourceId: 'entity-42',
        batchId: 'batch-7',
      })

      expect(putCalls[0].data.data.resourceId).toBe('entity-42')
      expect(putCalls[0].data.data.batchId).toBe('batch-7')
    })

    it('defaults severity to error', async () => {
      const { env, putCalls } = createMockEnv()

      await createViolation(env, {
        domain: 'd',
        constraintId: null,
        text: 'test',
      })

      expect(putCalls[0].data.data.severity).toBe('error')
    })

    it('accepts custom severity', async () => {
      const { env, putCalls } = createMockEnv()

      await createViolation(env, {
        domain: 'd',
        constraintId: null,
        text: 'test',
        severity: 'warning',
      })

      expect(putCalls[0].data.data.severity).toBe('warning')
    })
  })

  describe('createFailure', () => {
    it('creates an EntityDB DO and indexes it in the Registry', async () => {
      const { env, putCalls, indexCalls } = createMockEnv()

      const id = await createFailure(env, {
        domain: 'support',
        failureType: 'extraction',
        reason: 'Could not parse the input as FORML2.',
      })

      expect(id).toBe('test-uuid-1234')

      // EntityDB stub.put was called with correct entity shape
      expect(putCalls).toHaveLength(1)
      expect(putCalls[0].data.id).toBe('test-uuid-1234')
      expect(putCalls[0].data.type).toBe('Failure')
      expect(putCalls[0].data.data.domain).toBe('support')
      expect(putCalls[0].data.data.failureType).toBe('extraction')
      expect(putCalls[0].data.data.reason).toBe('Could not parse the input as FORML2.')
      expect(putCalls[0].data.data.severity).toBe('error')
      expect(putCalls[0].data.data.occurredAt).toBeDefined()
      expect(putCalls[0].data.data.input).toBeNull()

      // Registry indexEntity was called
      expect(indexCalls).toHaveLength(1)
      expect(indexCalls[0].nounType).toBe('Failure')
      expect(indexCalls[0].entityId).toBe('test-uuid-1234')
      expect(indexCalls[0].domain).toBe('support')
    })

    it('handles null domain gracefully', async () => {
      const { env, putCalls, indexCalls } = createMockEnv()

      await createFailure(env, {
        domain: null,
        failureType: 'parse',
        reason: 'Malformed JSON.',
      })

      expect(putCalls[0].data.data.domain).toBeNull()
      // Domain indexed as empty string when null
      expect(indexCalls[0].domain).toBe('')
    })

    it('stores input text when provided', async () => {
      const { env, putCalls } = createMockEnv()

      await createFailure(env, {
        domain: 'test',
        failureType: 'parse',
        reason: 'Invalid syntax.',
        input: 'Customer haz Name',
      })

      expect(putCalls[0].data.data.input).toBe('Customer haz Name')
    })
  })

  describe('persistViolations', () => {
    it('creates multiple violation entities and returns their ids', async () => {
      let counter = 0
      vi.stubGlobal('crypto', { randomUUID: vi.fn(() => `uuid-${++counter}`) })

      const { env, putCalls, indexCalls } = createMockEnv()

      const ids = await persistViolations(env, [
        { domain: 'd', constraintId: 'c1', text: 'v1' },
        { domain: 'd', constraintId: 'c2', text: 'v2' },
        { domain: 'd', constraintId: 'c3', text: 'v3' },
      ])

      expect(ids).toHaveLength(3)
      expect(putCalls).toHaveLength(3)
      expect(indexCalls).toHaveLength(3)
    })

    it('returns only successful ids when some fail', async () => {
      let counter = 0
      vi.stubGlobal('crypto', { randomUUID: vi.fn(() => `uuid-${++counter}`) })

      // Build an env where the second put throws
      let putCallCount = 0
      const mockStub = () => ({
        put: vi.fn(async (data: any) => {
          putCallCount++
          if (putCallCount === 2) throw new Error('DO unavailable')
          return { id: data.id, version: 1 }
        }),
      })
      const env = {
        ENTITY_DB: {
          idFromName: vi.fn((name: string) => name),
          get: vi.fn(() => mockStub()),
        },
        REGISTRY_DB: {
          idFromName: vi.fn(() => 'global'),
          get: vi.fn(() => ({
            indexEntity: vi.fn(async () => {}),
          })),
        },
        DOMAIN_DB: { idFromName: vi.fn(), get: vi.fn() },
        ENVIRONMENT: 'test',
      } as unknown as Env

      const ids = await persistViolations(env, [
        { domain: 'd', constraintId: 'c1', text: 'ok' },
        { domain: 'd', constraintId: 'c2', text: 'will fail' },
        { domain: 'd', constraintId: 'c3', text: 'ok' },
      ])

      // Only the successful ones
      expect(ids).toHaveLength(2)
    })
  })
})
