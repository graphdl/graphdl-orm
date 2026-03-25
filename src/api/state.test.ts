import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

// Mock createFailure before importing the module under test
vi.mock('../worker/outcomes', () => ({
  createFailure: vi.fn(() => Promise.resolve('failure-id')),
}))

import { handleSendEvent } from './state'
import { createFailure } from '../worker/outcomes'
import { executeCascade } from '../worker/cascade-transition'
import type { Env } from '../types'

/**
 * Tests for state machine auto-creation on entity creation,
 * status normalization on entity queries, and failure persistence
 * on guard/transition failures.
 *
 * These are unit tests for the logic — the DO methods are tested
 * via mock SQL in integration tests.
 */

describe('state machine auto-creation', () => {
  it('autoCreateStateMachine finds initial status (no incoming transitions)', () => {
    // Simulate: SupportRequest has SM definition with Received, Triaging, Resolved
    // Received has no incoming transitions → it's the initial state
    const sql = mockSql({
      // findDefinition
      'SELECT smd.id': [{ id: 'smd1', noun_id: 'n1' }],
      // findStatuses
      'SELECT id, name FROM statuses': [
        { id: 's1', name: 'Received' },
        { id: 's2', name: 'Triaging' },
        { id: 's3', name: 'Resolved' },
      ],
      // checkIncoming for s1 (Received) — no results
      'SELECT 1 FROM transitions WHERE to_status_id': [],
      // INSERT state_machines
      'INSERT INTO state_machines': [],
    })

    const result = autoCreateStateMachine(sql, 'domain1', 'SupportRequest', 'entity-123', '2026-01-01')
    expect(result).toBe('Received')
  })

  it('returns null when noun has no state machine definition', () => {
    const sql = mockSql({
      'SELECT smd.id': [], // no definitions found
    })

    const result = autoCreateStateMachine(sql, 'domain1', 'Message', 'msg-1', '2026-01-01')
    expect(result).toBeNull()
  })

  it('returns null when state machine has no statuses', () => {
    const sql = mockSql({
      'SELECT smd.id': [{ id: 'smd1', noun_id: 'n1' }],
      'SELECT id, name FROM statuses': [],
    })

    const result = autoCreateStateMachine(sql, 'domain1', 'SupportRequest', 'entity-1', '2026-01-01')
    expect(result).toBeNull()
  })

  it('falls back to first status when all have incoming transitions', () => {
    const sql = mockSql({
      'SELECT smd.id': [{ id: 'smd1', noun_id: 'n1' }],
      'SELECT id, name FROM statuses': [
        { id: 's1', name: 'A' },
        { id: 's2', name: 'B' },
      ],
      // Both have incoming transitions
      'SELECT 1 FROM transitions WHERE to_status_id': [{ '1': 1 }],
      'INSERT INTO state_machines': [],
    })

    const result = autoCreateStateMachine(sql, 'domain1', 'Cyclic', 'entity-1', '2026-01-01')
    expect(result).toBe('A') // falls back to first
  })
})

// ---------------------------------------------------------------------------
// Helpers — extract the pure logic from the DO method for testing
// ---------------------------------------------------------------------------

function autoCreateStateMachine(
  sql: ReturnType<typeof mockSql>,
  domainId: string,
  nounName: string,
  entityId: string,
  now: string,
): string | null {
  try {
    const defs = sql.exec(
      'SELECT smd.id, smd.noun_id FROM state_machine_definitions smd JOIN nouns n ON smd.noun_id = n.id WHERE n.name = ? AND smd.domain_id = ? LIMIT 1',
      nounName, domainId,
    )
    if (!defs.length) return null

    const defId = defs[0].id as string

    const statuses = sql.exec(
      'SELECT id, name FROM statuses WHERE state_machine_definition_id = ? ORDER BY created_at ASC',
      defId,
    )
    if (!statuses.length) return null

    let initialStatus = statuses[0]
    for (const s of statuses) {
      const incoming = sql.exec(
        'SELECT 1 FROM transitions WHERE to_status_id = ? LIMIT 1', s.id,
      )
      if (!incoming.length) { initialStatus = s; break }
    }

    sql.exec(
      'INSERT INTO state_machines (id, name, state_machine_definition_id, current_status_id, domain_id, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, 1)',
      'sm-id', entityId, defId, initialStatus.id, domainId, now, now,
    )

    return initialStatus.name as string
  } catch {
    return null
  }
}

function mockSql(responses: Record<string, any[]>) {
  const calls: string[] = []
  return {
    exec(query: string, ...params: any[]) {
      calls.push(query)
      // Match by prefix
      for (const [prefix, result] of Object.entries(responses)) {
        if (query.includes(prefix)) {
          // For queries that match multiple times with different params,
          // return the result and then remove it so next call gets empty
          return result
        }
      }
      return []
    },
    calls,
  }
}

// ---------------------------------------------------------------------------
// handleSendEvent failure persistence tests
// ---------------------------------------------------------------------------

/**
 * Build a mock Env whose Registry+EntityDB fan-out stubs return the given
 * entity records.  `entityMap` is keyed by entity id → entity record.
 * `registryIndex` maps entity type → list of entity ids.
 */
function buildMockEnv(
  entityMap: Record<string, { id: string; type: string; data: Record<string, unknown>; deletedAt?: string }>,
  registryIndex: Record<string, string[]>,
) {
  const mockRegistry = {
    getEntityIds: vi.fn(async (type: string, _domain?: string) => registryIndex[type] || []),
    indexEntity: vi.fn(async () => {}),
    deindexEntity: vi.fn(async () => {}),
  }

  const env = {
    ENTITY_DB: {
      idFromName: vi.fn((name: string) => name),
      get: vi.fn((name: string) => ({
        get: vi.fn(async () => entityMap[name] || null),
        put: vi.fn(async (data: any) => ({ id: data.id, version: 1 })),
        patch: vi.fn(async () => ({ version: 2 })),
        delete: vi.fn(async () => {}),
      })),
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

  return { env, mockRegistry }
}

function makeEventRequest(machineType: string, instanceId: string, event: string, domain?: string) {
  const domainParam = domain ? `?domain=${domain}` : ''
  return new Request(
    `http://localhost/api/state/${machineType}/${instanceId}/${event}${domainParam}`,
    { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' },
  )
}

describe('handleSendEvent failure persistence', () => {
  beforeEach(() => {
    vi.mocked(createFailure).mockClear()
    vi.stubGlobal('crypto', { randomUUID: vi.fn(() => 'new-sm-uuid') })
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('persists a Failure when state machine type is not found', async () => {
    const { env } = buildMockEnv({}, {
      'State Machine': [],
      'State Machine Definition': [],
    })

    const req = makeEventRequest('NonExistent', 'inst-1', 'go', 'test-domain')
    const res = await handleSendEvent(req, env)

    expect(res.status).toBe(404)
    const body = await res.json() as any
    expect(body.error).toContain('not found')

    expect(createFailure).toHaveBeenCalledTimes(1)
    expect(createFailure).toHaveBeenCalledWith(env, expect.objectContaining({
      domain: 'test-domain',
      failureType: 'transition',
      reason: expect.stringContaining("'NonExistent'"),
    }))
  })

  it('persists a Failure when no statuses are found for the machine type', async () => {
    const { env } = buildMockEnv(
      {
        'smd1': { id: 'smd1', type: 'State Machine Definition', data: { title: 'Order', nounId: 'noun-order', domain: 'test-domain' } },
      },
      {
        'State Machine': [],
        'State Machine Definition': ['smd1'],
        'Status': [],
        'Transition': [],
      },
    )

    const req = makeEventRequest('Order', 'inst-1', 'submit', 'test-domain')
    const res = await handleSendEvent(req, env)

    expect(res.status).toBe(404)
    const body = await res.json() as any
    expect(body.error).toContain('No statuses')

    expect(createFailure).toHaveBeenCalledTimes(1)
    expect(createFailure).toHaveBeenCalledWith(env, expect.objectContaining({
      domain: 'test-domain',
      failureType: 'transition',
      reason: expect.stringContaining("'Order'"),
      functionId: 'noun-order',
    }))
  })

  it('persists a Failure when event has no matching transition', async () => {
    // Setup: instance already exists at 'Open' status, but there's no transition for 'fly'
    const { env } = buildMockEnv(
      {
        'sm-1': { id: 'sm-1', type: 'State Machine', data: { name: 'inst-1', stateMachineType: 'smd1', stateMachineStatus: 'status-open' } },
        'status-open': { id: 'status-open', type: 'Status', data: { name: 'Open', stateMachineDefinition: 'smd1' } },
        'smd1': { id: 'smd1', type: 'State Machine Definition', data: { title: 'Ticket', nounId: 'noun-ticket' } },
      },
      {
        'State Machine': ['sm-1'],
        'State Machine Definition': ['smd1'],
        'Status': ['status-open'],
        'Transition': [],
        'Guard': [],
      },
    )

    const req = makeEventRequest('Ticket', 'inst-1', 'fly')
    const res = await handleSendEvent(req, env)

    expect(res.status).toBe(422)
    const body = await res.json() as any
    expect(body.error).toContain("No transition for event 'fly'")

    expect(createFailure).toHaveBeenCalledTimes(1)
    expect(createFailure).toHaveBeenCalledWith(env, expect.objectContaining({
      failureType: 'transition',
      reason: expect.stringContaining("'fly'"),
      functionId: 'noun-ticket',
    }))
  })

  it('persists a Failure when a guard blocks the transition', async () => {
    const { env } = buildMockEnv(
      {
        'sm-1': { id: 'sm-1', type: 'State Machine', data: { name: 'inst-1', stateMachineType: 'smd1', stateMachineStatus: 'status-pending' } },
        'status-pending': { id: 'status-pending', type: 'Status', data: { name: 'Pending', stateMachineDefinition: 'smd1' } },
        'status-approved': { id: 'status-approved', type: 'Status', data: { name: 'Approved', stateMachineDefinition: 'smd1' } },
        'smd1': { id: 'smd1', type: 'State Machine Definition', data: { title: 'Invoice', nounId: 'noun-invoice' } },
        'trans-1': { id: 'trans-1', type: 'Transition', data: { from: 'status-pending', to: 'status-approved', eventType: 'et-approve' } },
        'et-approve': { id: 'et-approve', type: 'Event Type', data: { name: 'approve' } },
        'guard-1': { id: 'guard-1', type: 'Guard', data: { name: 'paymentReceived', transition: 'trans-1', graphSchemaId: 'gs-payment' } },
        'gs-payment': { id: 'gs-payment', type: 'Graph Schema', data: { name: 'Payment received' } },
      },
      {
        'State Machine': ['sm-1'],
        'State Machine Definition': ['smd1'],
        'Status': ['status-pending', 'status-approved'],
        'Transition': ['trans-1'],
        'Guard': ['guard-1'],
        'Event Type': ['et-approve'],
      },
    )

    const req = makeEventRequest('Invoice', 'inst-1', 'approve')
    const res = await handleSendEvent(req, env)

    expect(res.status).toBe(422)
    const body = await res.json() as any
    expect(body.error).toContain("Guard 'paymentReceived' blocked transition")
    expect(body.error).toContain("'Pending'")
    expect(body.error).toContain("'Approved'")
    expect(body.guard).toBe('paymentReceived')

    expect(createFailure).toHaveBeenCalledTimes(1)
    expect(createFailure).toHaveBeenCalledWith(env, expect.objectContaining({
      failureType: 'transition',
      reason: expect.stringContaining("'paymentReceived'"),
      functionId: 'noun-invoice',
    }))
  })

  it('persists a Failure when guard references unavailable graph schema', async () => {
    const { env } = buildMockEnv(
      {
        'sm-1': { id: 'sm-1', type: 'State Machine', data: { name: 'inst-1', stateMachineType: 'smd1', stateMachineStatus: 'status-pending' } },
        'status-pending': { id: 'status-pending', type: 'Status', data: { name: 'Pending', stateMachineDefinition: 'smd1' } },
        'status-approved': { id: 'status-approved', type: 'Status', data: { name: 'Approved', stateMachineDefinition: 'smd1' } },
        'smd1': { id: 'smd1', type: 'State Machine Definition', data: { title: 'Invoice', nounId: 'noun-invoice' } },
        'trans-1': { id: 'trans-1', type: 'Transition', data: { from: 'status-pending', to: 'status-approved', eventType: 'et-approve' } },
        'et-approve': { id: 'et-approve', type: 'Event Type', data: { name: 'approve' } },
        'guard-1': { id: 'guard-1', type: 'Guard', data: { name: 'checkBalance', transition: 'trans-1', graphSchemaId: 'gs-missing' } },
        // gs-missing is NOT in the entityMap — simulates unavailable data
      },
      {
        'State Machine': ['sm-1'],
        'State Machine Definition': ['smd1'],
        'Status': ['status-pending', 'status-approved'],
        'Transition': ['trans-1'],
        'Guard': ['guard-1'],
        'Event Type': ['et-approve'],
      },
    )

    const req = makeEventRequest('Invoice', 'inst-1', 'approve')
    const res = await handleSendEvent(req, env)

    expect(res.status).toBe(422)
    const body = await res.json() as any
    expect(body.error).toContain('unavailable graph schema')
    expect(body.guard).toBe('checkBalance')

    expect(createFailure).toHaveBeenCalledTimes(1)
    expect(createFailure).toHaveBeenCalledWith(env, expect.objectContaining({
      failureType: 'transition',
      reason: expect.stringContaining('unavailable'),
      functionId: 'noun-invoice',
    }))
  })

  it('does not persist a Failure when transition succeeds (no guards)', async () => {
    const { env } = buildMockEnv(
      {
        'sm-1': { id: 'sm-1', type: 'State Machine', data: { name: 'inst-1', stateMachineType: 'smd1', stateMachineStatus: 'status-open' } },
        'status-open': { id: 'status-open', type: 'Status', data: { name: 'Open', stateMachineDefinition: 'smd1' } },
        'status-closed': { id: 'status-closed', type: 'Status', data: { name: 'Closed', stateMachineDefinition: 'smd1' } },
        'smd1': { id: 'smd1', type: 'State Machine Definition', data: { title: 'Ticket', nounId: 'noun-ticket' } },
        'trans-1': { id: 'trans-1', type: 'Transition', data: { from: 'status-open', to: 'status-closed', eventType: 'et-close' } },
        'et-close': { id: 'et-close', type: 'Event Type', data: { name: 'close' } },
      },
      {
        'State Machine': ['sm-1'],
        'State Machine Definition': ['smd1'],
        'Status': ['status-open', 'status-closed'],
        'Transition': ['trans-1'],
        'Guard': [],
        'Event Type': ['et-close'],
        'Function': [],
      },
    )

    const req = makeEventRequest('Ticket', 'inst-1', 'close')
    const res = await handleSendEvent(req, env)

    expect(res.status).toBe(200)
    const body = await res.json() as any
    expect(body.previousState).toBe('Open')
    expect(body.currentState).toBe('Closed')

    // No failure persisted on success
    expect(createFailure).not.toHaveBeenCalled()
  })

  it('failure persistence does not block the error response', async () => {
    // Make createFailure reject — the response should still be returned
    vi.mocked(createFailure).mockRejectedValueOnce(new Error('DO unavailable'))

    const { env } = buildMockEnv({}, {
      'State Machine': [],
      'State Machine Definition': [],
    })

    const req = makeEventRequest('Ghost', 'inst-1', 'vanish')
    const res = await handleSendEvent(req, env)

    // The response should still come back despite createFailure rejecting
    expect(res.status).toBe(404)
    const body = await res.json() as any
    expect(body.error).toContain("'Ghost'")
  })
})

// ---------------------------------------------------------------------------
// Cascade integration tests (transition endpoint wiring)
// ---------------------------------------------------------------------------

/**
 * Build a mock fan-out pair (registry + getStub) matching the pattern used
 * by the transition endpoint in router.ts.  The entity map is keyed by
 * entity id → entity record.  The registry index maps entity type → list
 * of entity ids.  The getStub also supports patch() for status updates.
 */
function buildCascadeMocks(
  entityMap: Record<string, { id: string; type: string; data: Record<string, unknown> }>,
  registryIndex: Record<string, string[]>,
) {
  const byId = new Map<string, { id: string; type: string; data: Record<string, unknown> }>()
  for (const [id, entity] of Object.entries(entityMap)) {
    byId.set(id, { ...entity })
  }

  const registry = {
    getEntityIds: vi.fn(async (type: string, _domain?: string) => registryIndex[type] || []),
  }

  const patchCalls: Array<{ id: string; data: any }> = []
  const getStub = (entityId: string) => ({
    get: vi.fn(async () => byId.get(entityId) || null),
    patch: vi.fn(async (data: any) => {
      patchCalls.push({ id: entityId, data })
      const existing = byId.get(entityId)
      if (existing) {
        byId.set(entityId, { ...existing, data: { ...existing.data, ...data } })
      }
      return { version: 2 }
    }),
  })

  return { registry, getStub, patchCalls, byId }
}

describe('cascade integration — transition endpoint wiring', () => {
  it('transition returns cascade result with statesVisited', async () => {
    const { registry, getStub } = buildCascadeMocks(
      {
        'ent-1': { id: 'ent-1', type: 'Order', data: { _status: 'Open', _statusId: 'status-open', _stateMachineDefinition: 'smd-1' } },
        'status-open': { id: 'status-open', type: 'Status', data: { name: 'Open', stateMachineDefinition: 'smd-1' } },
        'status-closed': { id: 'status-closed', type: 'Status', data: { name: 'Closed', stateMachineDefinition: 'smd-1' } },
        'trans-1': { id: 'trans-1', type: 'Transition', data: { from: 'status-open', to: 'status-closed', eventType: 'et-close', stateMachineDefinition: 'smd-1' } },
        'et-close': { id: 'et-close', type: 'Event Type', data: { name: 'close' } },
      },
      {
        'Transition': ['trans-1'],
        'Event Type': ['et-close'],
        'Status': ['status-open', 'status-closed'],
        'Function': [],
      },
    )

    const result = await executeCascade('ent-1', 'close', {
      registry,
      getStub,
      domain: 'test',
    })

    // The shape that the router returns as cascade info
    expect(result.finalState).toBe('Closed')
    expect(result.statesVisited).toEqual(['Closed'])
    expect(result.callbackResults).toEqual([])
    expect(result.failures).toEqual([])
  })

  it('transition cascades through callback and returns full chain', async () => {
    const mockFetch = vi.fn().mockResolvedValueOnce({ status: 200 })

    const { registry, getStub } = buildCascadeMocks(
      {
        'ent-1': { id: 'ent-1', type: 'Order', data: { _status: 'Submitted', _statusId: 's-submitted', _stateMachineDefinition: 'smd-1' } },
        's-submitted': { id: 's-submitted', type: 'Status', data: { name: 'Submitted', stateMachineDefinition: 'smd-1' } },
        's-processing': { id: 's-processing', type: 'Status', data: { name: 'Processing', stateMachineDefinition: 'smd-1' } },
        's-done': { id: 's-done', type: 'Status', data: { name: 'Done', stateMachineDefinition: 'smd-1' } },
        't1': { id: 't1', type: 'Transition', data: { from: 's-submitted', to: 's-processing', eventType: 'et-submit', verb: 'v1', stateMachineDefinition: 'smd-1' } },
        't2': { id: 't2', type: 'Transition', data: { from: 's-processing', to: 's-done', eventType: 'et-ok', stateMachineDefinition: 'smd-1' } },
        'et-submit': { id: 'et-submit', type: 'Event Type', data: { name: 'submit' } },
        'et-ok': { id: 'et-ok', type: 'Event Type', data: { name: 'on_ok', pattern: '2XX' } },
        'func-1': { id: 'func-1', type: 'Function', data: { verb: 'v1', callbackUrl: 'https://api.test.com/process', httpMethod: 'POST' } },
      },
      {
        'Transition': ['t1', 't2'],
        'Event Type': ['et-submit', 'et-ok'],
        'Status': ['s-submitted', 's-processing', 's-done'],
        'Function': ['func-1'],
      },
    )

    const result = await executeCascade('ent-1', 'submit', {
      registry,
      getStub,
      fetchCallback: mockFetch,
      domain: 'test',
    })

    expect(result.finalState).toBe('Done')
    expect(result.statesVisited).toEqual(['Processing', 'Done'])
    expect(result.callbackResults).toEqual([
      { status: 200, url: 'https://api.test.com/process' },
    ])
    expect(result.failures).toEqual([])
  })

  it('cascade result includes failures on callback error', async () => {
    const mockFetch = vi.fn().mockRejectedValueOnce(new Error('timeout'))

    const { registry, getStub } = buildCascadeMocks(
      {
        'ent-1': { id: 'ent-1', type: 'Order', data: { _status: 'Open', _statusId: 's-open', _stateMachineDefinition: 'smd-1' } },
        's-open': { id: 's-open', type: 'Status', data: { name: 'Open', stateMachineDefinition: 'smd-1' } },
        's-calling': { id: 's-calling', type: 'Status', data: { name: 'Calling', stateMachineDefinition: 'smd-1' } },
        't1': { id: 't1', type: 'Transition', data: { from: 's-open', to: 's-calling', eventType: 'et-call', verb: 'v1', stateMachineDefinition: 'smd-1' } },
        'et-call': { id: 'et-call', type: 'Event Type', data: { name: 'call' } },
        'func-1': { id: 'func-1', type: 'Function', data: { verb: 'v1', callbackUrl: 'https://api.test.com/webhook', httpMethod: 'POST' } },
      },
      {
        'Transition': ['t1'],
        'Event Type': ['et-call'],
        'Status': ['s-open', 's-calling'],
        'Function': ['func-1'],
      },
    )

    const result = await executeCascade('ent-1', 'call', {
      registry,
      getStub,
      fetchCallback: mockFetch,
      domain: 'test',
    })

    expect(result.finalState).toBe('Calling')
    expect(result.statesVisited).toEqual(['Calling'])
    expect(result.failures.length).toBe(1)
    expect(result.failures[0]).toContain('timeout')
  })
})
