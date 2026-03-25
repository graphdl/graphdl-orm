import { describe, it, expect, vi } from 'vitest'
import { matchPattern, executeCascade, type CascadeContext } from './cascade-transition'
import type { EntityRecord, RegistryReadStub, EntityReadStub } from '../api/entity-routes'

// ---------------------------------------------------------------------------
// Mock helpers
// ---------------------------------------------------------------------------

function mockFanOut(
  data: Record<string, Array<{ id: string; [k: string]: any }>>,
) {
  const byId = new Map<string, EntityRecord>()
  const idsByType = new Map<string, string[]>()

  for (const [type, entities] of Object.entries(data)) {
    const ids: string[] = []
    for (const raw of entities) {
      const { id, ...rest } = raw
      const record: EntityRecord = { id, type, data: rest, version: 1 }
      byId.set(id, record)
      ids.push(id)
    }
    idsByType.set(type, ids)
  }

  const patchCalls: Array<{ id: string; data: any }> = []

  const registry: RegistryReadStub = {
    getEntityIds: vi.fn(async (entityType: string, _domain?: string) => {
      return idsByType.get(entityType) || []
    }),
  }

  const getStub = (entityId: string) => ({
    get: vi.fn(async () => {
      const e = byId.get(entityId)
      return e || null
    }),
    patch: vi.fn(async (data: any) => {
      patchCalls.push({ id: entityId, data })
      // Also update the in-memory record so subsequent reads see the change
      const existing = byId.get(entityId)
      if (existing) {
        byId.set(entityId, { ...existing, data: { ...existing.data, ...data } })
      }
      return { version: 2 }
    }),
  })

  return { registry, getStub, patchCalls, byId }
}

// ---------------------------------------------------------------------------
// matchPattern tests
// ---------------------------------------------------------------------------

describe('matchPattern', () => {
  it('matches 200 against 2XX', () => {
    expect(matchPattern(200, '2XX')).toBe(true)
  })

  it('matches 404 against 4XX', () => {
    expect(matchPattern(404, '4XX')).toBe(true)
  })

  it('matches 500 against 5XX', () => {
    expect(matchPattern(500, '5XX')).toBe(true)
  })

  it('matches 200 against *', () => {
    expect(matchPattern(200, '*')).toBe(true)
  })

  it('does not match 200 against 201', () => {
    expect(matchPattern(200, '201')).toBe(false)
  })

  it('matches exact code 200 against 200', () => {
    expect(matchPattern(200, '200')).toBe(true)
  })

  it('does not match 301 against 2XX', () => {
    expect(matchPattern(301, '2XX')).toBe(false)
  })

  it('matches 499 against 4XX', () => {
    expect(matchPattern(499, '4XX')).toBe(true)
  })

  it('handles lowercase x in pattern', () => {
    expect(matchPattern(200, '2xx')).toBe(true)
  })
})

// ---------------------------------------------------------------------------
// executeCascade tests
// ---------------------------------------------------------------------------

describe('executeCascade', () => {
  it('single transition, no callback — fires once, stops', async () => {
    const { registry, getStub, patchCalls } = mockFanOut({
      // Entity (the instance being transitioned)
      'SupportRequest': [
        { id: 'ent-1', _status: 'Open', _statusId: 'status-open', _stateMachineDefinition: 'smd-1' },
      ],
      'Status': [
        { id: 'status-open', name: 'Open', stateMachineDefinition: 'smd-1' },
        { id: 'status-closed', name: 'Closed', stateMachineDefinition: 'smd-1' },
      ],
      'Transition': [
        { id: 't1', from: 'status-open', to: 'status-closed', eventType: 'et-close', stateMachineDefinition: 'smd-1' },
      ],
      'Event Type': [
        { id: 'et-close', name: 'close' },
      ],
      'Function': [],
    })

    const result = await executeCascade('ent-1', 'close', {
      registry,
      getStub,
      domain: 'test',
    })

    expect(result.finalState).toBe('Closed')
    expect(result.statesVisited).toEqual(['Closed'])
    expect(result.callbackResults).toEqual([])
    expect(result.failures).toEqual([])
    // Entity was patched with new status
    expect(patchCalls).toHaveLength(1)
    expect(patchCalls[0].data._status).toBe('Closed')
  })

  it('transition with callback returning 200, outgoing 2XX pattern — cascades', async () => {
    const mockFetch = vi.fn()
      .mockResolvedValueOnce({ status: 200 }) // first callback

    const { registry, getStub } = mockFanOut({
      'SupportRequest': [
        { id: 'ent-1', _status: 'Submitted', _statusId: 'status-submitted', _stateMachineDefinition: 'smd-1' },
      ],
      'Status': [
        { id: 'status-submitted', name: 'Submitted', stateMachineDefinition: 'smd-1' },
        { id: 'status-processing', name: 'Processing', stateMachineDefinition: 'smd-1' },
        { id: 'status-done', name: 'Done', stateMachineDefinition: 'smd-1' },
      ],
      'Transition': [
        // submit → Processing (has verb with callback)
        { id: 't1', from: 'status-submitted', to: 'status-processing', eventType: 'et-submit', verb: 'verb-1', stateMachineDefinition: 'smd-1' },
        // Processing → Done (triggered by 2XX pattern)
        { id: 't2', from: 'status-processing', to: 'status-done', eventType: 'et-success', stateMachineDefinition: 'smd-1' },
      ],
      'Event Type': [
        { id: 'et-submit', name: 'submit' },
        { id: 'et-success', name: 'on_success', pattern: '2XX' },
      ],
      // Verb entity with callbackUrl directly
      'Verb': [
        { id: 'verb-1', name: 'processOrder', callbackUrl: 'https://api.example.com/process' },
      ],
      'Function': [
        { id: 'func-1', verb: 'verb-1', callbackUrl: 'https://api.example.com/process', httpMethod: 'POST' },
      ],
    })

    const result = await executeCascade('ent-1', 'submit', {
      registry,
      getStub,
      fetchCallback: mockFetch,
      domain: 'test',
    })

    expect(result.finalState).toBe('Done')
    expect(result.statesVisited).toEqual(['Processing', 'Done'])
    expect(result.callbackResults).toHaveLength(1)
    expect(result.callbackResults[0].status).toBe(200)
    expect(result.callbackResults[0].url).toBe('https://api.example.com/process')
    expect(result.failures).toEqual([])
  })

  it('callback error — stops and records failure', async () => {
    const mockFetch = vi.fn().mockRejectedValueOnce(new Error('Connection refused'))

    const { registry, getStub } = mockFanOut({
      'SupportRequest': [
        { id: 'ent-1', _status: 'Open', _statusId: 'status-open', _stateMachineDefinition: 'smd-1' },
      ],
      'Status': [
        { id: 'status-open', name: 'Open', stateMachineDefinition: 'smd-1' },
        { id: 'status-calling', name: 'Calling', stateMachineDefinition: 'smd-1' },
      ],
      'Transition': [
        { id: 't1', from: 'status-open', to: 'status-calling', eventType: 'et-call', verb: 'verb-1', stateMachineDefinition: 'smd-1' },
      ],
      'Event Type': [
        { id: 'et-call', name: 'call' },
      ],
      'Function': [
        { id: 'func-1', verb: 'verb-1', callbackUrl: 'https://api.example.com/webhook', httpMethod: 'POST' },
      ],
    })

    const result = await executeCascade('ent-1', 'call', {
      registry,
      getStub,
      fetchCallback: mockFetch,
      domain: 'test',
    })

    expect(result.finalState).toBe('Calling')
    expect(result.statesVisited).toEqual(['Calling'])
    expect(result.callbackResults).toEqual([])
    expect(result.failures).toHaveLength(1)
    expect(result.failures[0]).toContain('Connection refused')
  })

  it('max depth exceeded — stops with failure message', async () => {
    // Build a cycle: A → B → A → B → ... with callbacks always returning 200
    const mockFetch = vi.fn().mockResolvedValue({ status: 200 })

    const { registry, getStub } = mockFanOut({
      'SupportRequest': [
        { id: 'ent-1', _status: 'A', _statusId: 'status-a', _stateMachineDefinition: 'smd-1' },
      ],
      'Status': [
        { id: 'status-a', name: 'A', stateMachineDefinition: 'smd-1' },
        { id: 'status-b', name: 'B', stateMachineDefinition: 'smd-1' },
      ],
      'Transition': [
        { id: 't1', from: 'status-a', to: 'status-b', eventType: 'et-go', verb: 'verb-1', stateMachineDefinition: 'smd-1' },
        { id: 't2', from: 'status-b', to: 'status-a', eventType: 'et-back', verb: 'verb-1', stateMachineDefinition: 'smd-1' },
      ],
      'Event Type': [
        { id: 'et-go', name: 'go', pattern: '2XX' },
        { id: 'et-back', name: 'back', pattern: '2XX' },
      ],
      'Function': [
        { id: 'func-1', verb: 'verb-1', callbackUrl: 'https://api.example.com/cycle', httpMethod: 'POST' },
      ],
    })

    const result = await executeCascade('ent-1', 'go', {
      registry,
      getStub,
      fetchCallback: mockFetch,
      domain: 'test',
      maxDepth: 3,
    })

    // Should stop after 3 iterations
    expect(result.statesVisited.length).toBe(3)
    expect(result.failures).toContain('Max cascade depth (3) reached')
  })

  it('no matching pattern — stops at new state', async () => {
    // Callback returns 500 but only 2XX pattern exists on outgoing transition
    const mockFetch = vi.fn().mockResolvedValueOnce({ status: 500 })

    const { registry, getStub } = mockFanOut({
      'SupportRequest': [
        { id: 'ent-1', _status: 'Pending', _statusId: 'status-pending', _stateMachineDefinition: 'smd-1' },
      ],
      'Status': [
        { id: 'status-pending', name: 'Pending', stateMachineDefinition: 'smd-1' },
        { id: 'status-processing', name: 'Processing', stateMachineDefinition: 'smd-1' },
        { id: 'status-done', name: 'Done', stateMachineDefinition: 'smd-1' },
      ],
      'Transition': [
        { id: 't1', from: 'status-pending', to: 'status-processing', eventType: 'et-process', verb: 'verb-1', stateMachineDefinition: 'smd-1' },
        // Only 2XX pattern on outgoing — won't match 500
        { id: 't2', from: 'status-processing', to: 'status-done', eventType: 'et-success', stateMachineDefinition: 'smd-1' },
      ],
      'Event Type': [
        { id: 'et-process', name: 'process' },
        { id: 'et-success', name: 'on_success', pattern: '2XX' },
      ],
      'Function': [
        { id: 'func-1', verb: 'verb-1', callbackUrl: 'https://api.example.com/process', httpMethod: 'POST' },
      ],
    })

    const result = await executeCascade('ent-1', 'process', {
      registry,
      getStub,
      fetchCallback: mockFetch,
      domain: 'test',
    })

    expect(result.finalState).toBe('Processing')
    expect(result.statesVisited).toEqual(['Processing'])
    expect(result.callbackResults).toHaveLength(1)
    expect(result.callbackResults[0].status).toBe(500)
    expect(result.failures).toEqual([])
  })

  it('entity not found — returns unknown with failure', async () => {
    const { registry, getStub } = mockFanOut({})

    const result = await executeCascade('nonexistent', 'go', {
      registry,
      getStub,
      domain: 'test',
    })

    expect(result.finalState).toBe('unknown')
    expect(result.failures).toContain('Entity not found')
  })

  it('multi-hop cascade: submit → Processing (200) → Done', async () => {
    // Same as the 2XX test but validates the full chain including intermediate states
    const mockFetch = vi.fn()
      .mockResolvedValueOnce({ status: 201 }) // callback from t1

    const { registry, getStub, patchCalls } = mockFanOut({
      'Order': [
        { id: 'order-1', _status: 'New', _statusId: 's-new', _stateMachineDefinition: 'smd-1' },
      ],
      'Status': [
        { id: 's-new', name: 'New', stateMachineDefinition: 'smd-1' },
        { id: 's-processing', name: 'Processing', stateMachineDefinition: 'smd-1' },
        { id: 's-complete', name: 'Complete', stateMachineDefinition: 'smd-1' },
      ],
      'Transition': [
        { id: 't1', from: 's-new', to: 's-processing', eventType: 'et-submit', verb: 'verb-1', stateMachineDefinition: 'smd-1' },
        { id: 't2', from: 's-processing', to: 's-complete', eventType: 'et-ok', stateMachineDefinition: 'smd-1' },
      ],
      'Event Type': [
        { id: 'et-submit', name: 'submit' },
        { id: 'et-ok', name: 'ok', pattern: '2XX' },
      ],
      'Function': [
        { id: 'func-1', verb: 'verb-1', callbackUrl: 'https://api.example.com/submit', httpMethod: 'POST' },
      ],
    })

    const result = await executeCascade('order-1', 'submit', {
      registry,
      getStub,
      fetchCallback: mockFetch,
      domain: 'test',
    })

    expect(result.finalState).toBe('Complete')
    expect(result.statesVisited).toEqual(['Processing', 'Complete'])
    expect(result.callbackResults).toEqual([{ status: 201, url: 'https://api.example.com/submit' }])
    // Two patches: first to Processing, then to Complete
    expect(patchCalls).toHaveLength(2)
  })

  it('callback returning 4XX matches 4XX pattern to fire error transition', async () => {
    const mockFetch = vi.fn().mockResolvedValueOnce({ status: 422 })

    const { registry, getStub } = mockFanOut({
      'Order': [
        { id: 'order-1', _status: 'New', _statusId: 's-new', _stateMachineDefinition: 'smd-1' },
      ],
      'Status': [
        { id: 's-new', name: 'New', stateMachineDefinition: 'smd-1' },
        { id: 's-validating', name: 'Validating', stateMachineDefinition: 'smd-1' },
        { id: 's-invalid', name: 'Invalid', stateMachineDefinition: 'smd-1' },
        { id: 's-valid', name: 'Valid', stateMachineDefinition: 'smd-1' },
      ],
      'Transition': [
        { id: 't1', from: 's-new', to: 's-validating', eventType: 'et-validate', verb: 'verb-1', stateMachineDefinition: 'smd-1' },
        { id: 't2', from: 's-validating', to: 's-invalid', eventType: 'et-fail', stateMachineDefinition: 'smd-1' },
        { id: 't3', from: 's-validating', to: 's-valid', eventType: 'et-pass', stateMachineDefinition: 'smd-1' },
      ],
      'Event Type': [
        { id: 'et-validate', name: 'validate' },
        { id: 'et-fail', name: 'on_fail', pattern: '4XX' },
        { id: 'et-pass', name: 'on_pass', pattern: '2XX' },
      ],
      'Function': [
        { id: 'func-1', verb: 'verb-1', callbackUrl: 'https://api.example.com/validate', httpMethod: 'POST' },
      ],
    })

    const result = await executeCascade('order-1', 'validate', {
      registry,
      getStub,
      fetchCallback: mockFetch,
      domain: 'test',
    })

    expect(result.finalState).toBe('Invalid')
    expect(result.statesVisited).toEqual(['Validating', 'Invalid'])
    expect(result.callbackResults[0].status).toBe(422)
  })
})
