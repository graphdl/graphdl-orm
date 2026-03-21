import { describe, it, expect, vi } from 'vitest'
import { getInitialState, getValidTransitions, applyTransition, type DomainDBStub } from './state-machine'

function mockDomainDB(data: Record<string, any[]>): DomainDBStub {
  return {
    findInCollection: vi.fn(async (collection: string, where: any) => {
      const all = data[collection] || []
      const filtered = all.filter((doc: any) => {
        for (const [key, cond] of Object.entries(where)) {
          if (typeof cond === 'object' && cond !== null && 'equals' in (cond as any)) {
            if (doc[key] !== (cond as any).equals) return false
          }
        }
        return true
      })
      return { docs: filtered, totalDocs: filtered.length }
    }),
  }
}

const SUPPORT_DATA = {
  nouns: [{ id: 'n1', name: 'SupportRequest' }],
  'state-machine-definitions': [{ id: 'smd1', noun: 'n1', title: 'Support Request Lifecycle' }],
  statuses: [
    { id: 's1', name: 'Received', stateMachineDefinition: 'smd1' },
    { id: 's2', name: 'Triaging', stateMachineDefinition: 'smd1' },
    { id: 's3', name: 'Resolved', stateMachineDefinition: 'smd1' },
  ],
  transitions: [
    { id: 't1', from: 's1', to: 's2', eventType: 'e1', stateMachineDefinition: 'smd1' },
    { id: 't2', from: 's2', to: 's3', eventType: 'e2', stateMachineDefinition: 'smd1' },
  ],
  'event-types': [
    { id: 'e1', name: 'triage' },
    { id: 'e2', name: 'resolve' },
  ],
}

describe('getInitialState', () => {
  it('returns initial status for noun with state machine', async () => {
    const db = mockDomainDB(SUPPORT_DATA)
    const result = await getInitialState(db, 'SupportRequest', 'domain1')
    expect(result).not.toBeNull()
    expect(result!.initialStatus).toBe('Received')
    expect(result!.definitionId).toBe('smd1')
  })

  it('returns null for noun without state machine', async () => {
    const db = mockDomainDB({ nouns: [{ id: 'n1', name: 'Customer' }], 'state-machine-definitions': [] })
    const result = await getInitialState(db, 'Customer', 'domain1')
    expect(result).toBeNull()
  })

  it('returns null for unknown noun', async () => {
    const db = mockDomainDB({ nouns: [] })
    const result = await getInitialState(db, 'Unknown', 'domain1')
    expect(result).toBeNull()
  })

  it('falls back to first status for cyclic machines', async () => {
    const db = mockDomainDB({
      nouns: [{ id: 'n1', name: 'Cyclic' }],
      'state-machine-definitions': [{ id: 'smd1', noun: 'n1', title: 'Cyclic' }],
      statuses: [
        { id: 's1', name: 'A', stateMachineDefinition: 'smd1' },
        { id: 's2', name: 'B', stateMachineDefinition: 'smd1' },
      ],
      transitions: [
        { id: 't1', from: 's1', to: 's2', eventType: 'e1', stateMachineDefinition: 'smd1' },
        { id: 't2', from: 's2', to: 's1', eventType: 'e2', stateMachineDefinition: 'smd1' },
      ],
      'event-types': [{ id: 'e1', name: 'next' }, { id: 'e2', name: 'back' }],
    })
    const result = await getInitialState(db, 'Cyclic', 'domain1')
    expect(result).not.toBeNull()
    expect(result!.initialStatus).toBe('A')
  })
})

describe('getValidTransitions', () => {
  it('returns valid transitions from current status', async () => {
    const db = mockDomainDB(SUPPORT_DATA)
    const transitions = await getValidTransitions(db, 'smd1', 's1')
    expect(transitions).toHaveLength(1)
    expect(transitions[0].event).toBe('triage')
    expect(transitions[0].targetStatus).toBe('Triaging')
  })

  it('returns empty for terminal status', async () => {
    const db = mockDomainDB(SUPPORT_DATA)
    const transitions = await getValidTransitions(db, 'smd1', 's3')
    expect(transitions).toHaveLength(0)
  })
})

describe('applyTransition', () => {
  it('transitions to valid target status', async () => {
    const db = mockDomainDB(SUPPORT_DATA)
    const result = await applyTransition(db, 'smd1', 's1', 'triage')
    expect(result).not.toBeNull()
    expect(result!.newStatus).toBe('Triaging')
    expect(result!.newStatusId).toBe('s2')
  })

  it('returns null for invalid event', async () => {
    const db = mockDomainDB(SUPPORT_DATA)
    const result = await applyTransition(db, 'smd1', 's1', 'resolve')
    expect(result).toBeNull()
  })

  it('returns null for invalid current status', async () => {
    const db = mockDomainDB(SUPPORT_DATA)
    const result = await applyTransition(db, 'smd1', 'nonexistent', 'triage')
    expect(result).toBeNull()
  })
})
