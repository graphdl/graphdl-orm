import { describe, it, expect, vi } from 'vitest'
import { getInitialState, getValidTransitions, applyTransition, type GetStubFn } from './state-machine'
import type { RegistryReadStub, EntityReadStub, EntityRecord } from '../api/entity-routes'

/**
 * Build a mock Registry + EntityDB fan-out pair from a flat record of
 * entity-type → entity[] data (same shape as the old DomainDB mock, but
 * keyed by entity type name instead of collection slug).
 */
function mockFanOut(data: Record<string, Array<{ id: string; [k: string]: any }>>) {
  // Flatten all entities into a by-id map for direct lookups
  const byId = new Map<string, EntityRecord>()
  const idsByType = new Map<string, string[]>()

  for (const [type, entities] of Object.entries(data)) {
    const ids: string[] = []
    for (const raw of entities) {
      const { id, ...rest } = raw
      const record: EntityRecord = {
        id,
        type,
        data: rest,
        version: 1,
      }
      byId.set(id, record)
      ids.push(id)
    }
    idsByType.set(type, ids)
  }

  const registry: RegistryReadStub = {
    getEntityIds: vi.fn(async (entityType: string, _domain?: string) => {
      return idsByType.get(entityType) || []
    }),
  }

  const getStub: GetStubFn = (entityId: string) => ({
    get: vi.fn(async () => byId.get(entityId) || null),
  })

  return { registry, getStub }
}

const SUPPORT_DATA = {
  'Noun': [{ id: 'n1', name: 'SupportRequest' }],
  'State Machine Definition': [{ id: 'smd1', noun: 'n1', title: 'Support Request Lifecycle' }],
  'Status': [
    { id: 's1', name: 'Received', stateMachineDefinition: 'smd1' },
    { id: 's2', name: 'Triaging', stateMachineDefinition: 'smd1' },
    { id: 's3', name: 'Resolved', stateMachineDefinition: 'smd1' },
  ],
  'Transition': [
    { id: 't1', from: 's1', to: 's2', eventType: 'e1', stateMachineDefinition: 'smd1' },
    { id: 't2', from: 's2', to: 's3', eventType: 'e2', stateMachineDefinition: 'smd1' },
  ],
  'Event Type': [
    { id: 'e1', name: 'triage' },
    { id: 'e2', name: 'resolve' },
  ],
}

describe('getInitialState', () => {
  it('returns initial status for noun with state machine', async () => {
    const { registry, getStub } = mockFanOut(SUPPORT_DATA)
    const result = await getInitialState(registry, getStub, 'SupportRequest', 'domain1')
    expect(result).not.toBeNull()
    expect(result!.initialStatus).toBe('Received')
    expect(result!.definitionId).toBe('smd1')
  })

  it('returns null for noun without state machine', async () => {
    const { registry, getStub } = mockFanOut({
      'Noun': [{ id: 'n1', name: 'Customer' }],
      'State Machine Definition': [],
    })
    const result = await getInitialState(registry, getStub, 'Customer', 'domain1')
    expect(result).toBeNull()
  })

  it('returns null for unknown noun', async () => {
    const { registry, getStub } = mockFanOut({ 'Noun': [] })
    const result = await getInitialState(registry, getStub, 'Unknown', 'domain1')
    expect(result).toBeNull()
  })

  it('selects status with isInitial flag over heuristic', async () => {
    // s1 has no incoming transitions (heuristic would pick it)
    // s2 has isInitial: true AND has an incoming transition (heuristic would skip it)
    // isInitial flag should win
    const { registry, getStub } = mockFanOut({
      'Noun': [{ id: 'n1', name: 'Ticket' }],
      'State Machine Definition': [{ id: 'smd1', noun: 'n1', title: 'Ticket Lifecycle' }],
      'Status': [
        { id: 's1', name: 'Open', stateMachineDefinition: 'smd1' },
        { id: 's2', name: 'Draft', stateMachineDefinition: 'smd1', isInitial: true },
        { id: 's3', name: 'Closed', stateMachineDefinition: 'smd1' },
      ],
      'Transition': [
        { id: 't1', from: 's2', to: 's1', eventType: 'e1', stateMachineDefinition: 'smd1' },
        { id: 't2', from: 's1', to: 's3', eventType: 'e2', stateMachineDefinition: 'smd1' },
        { id: 't3', from: 's3', to: 's2', eventType: 'e3', stateMachineDefinition: 'smd1' },
      ],
      'Event Type': [
        { id: 'e1', name: 'open' },
        { id: 'e2', name: 'close' },
        { id: 'e3', name: 'reopen' },
      ],
    })
    const result = await getInitialState(registry, getStub, 'Ticket', 'domain1')
    expect(result).not.toBeNull()
    expect(result!.initialStatus).toBe('Draft')
    expect(result!.initialStatusId).toBe('s2')
  })

  it('falls back to heuristic when no status has isInitial flag', async () => {
    // No status has isInitial — should use existing heuristic (no incoming transitions)
    const { registry, getStub } = mockFanOut(SUPPORT_DATA)
    const result = await getInitialState(registry, getStub, 'SupportRequest', 'domain1')
    expect(result).not.toBeNull()
    expect(result!.initialStatus).toBe('Received')
  })

  it('falls back to first status for cyclic machines', async () => {
    const { registry, getStub } = mockFanOut({
      'Noun': [{ id: 'n1', name: 'Cyclic' }],
      'State Machine Definition': [{ id: 'smd1', noun: 'n1', title: 'Cyclic' }],
      'Status': [
        { id: 's1', name: 'A', stateMachineDefinition: 'smd1' },
        { id: 's2', name: 'B', stateMachineDefinition: 'smd1' },
      ],
      'Transition': [
        { id: 't1', from: 's1', to: 's2', eventType: 'e1', stateMachineDefinition: 'smd1' },
        { id: 't2', from: 's2', to: 's1', eventType: 'e2', stateMachineDefinition: 'smd1' },
      ],
      'Event Type': [{ id: 'e1', name: 'next' }, { id: 'e2', name: 'back' }],
    })
    const result = await getInitialState(registry, getStub, 'Cyclic', 'domain1')
    expect(result).not.toBeNull()
    expect(result!.initialStatus).toBe('A')
  })
})

describe('getValidTransitions', () => {
  it('returns valid transitions from current status', async () => {
    const { registry, getStub } = mockFanOut(SUPPORT_DATA)
    const transitions = await getValidTransitions(registry, getStub, 'smd1', 's1')
    expect(transitions).toHaveLength(1)
    expect(transitions[0].event).toBe('triage')
    expect(transitions[0].targetStatus).toBe('Triaging')
  })

  it('returns empty for terminal status', async () => {
    const { registry, getStub } = mockFanOut(SUPPORT_DATA)
    const transitions = await getValidTransitions(registry, getStub, 'smd1', 's3')
    expect(transitions).toHaveLength(0)
  })
})

describe('applyTransition', () => {
  it('transitions to valid target status', async () => {
    const { registry, getStub } = mockFanOut(SUPPORT_DATA)
    const result = await applyTransition(registry, getStub, 'smd1', 's1', 'triage')
    expect(result).not.toBeNull()
    expect(result!.newStatus).toBe('Triaging')
    expect(result!.newStatusId).toBe('s2')
  })

  it('returns null for invalid event', async () => {
    const { registry, getStub } = mockFanOut(SUPPORT_DATA)
    const result = await applyTransition(registry, getStub, 'smd1', 's1', 'resolve')
    expect(result).toBeNull()
  })

  it('returns null for invalid current status', async () => {
    const { registry, getStub } = mockFanOut(SUPPORT_DATA)
    const result = await applyTransition(registry, getStub, 'smd1', 'nonexistent', 'triage')
    expect(result).toBeNull()
  })
})
