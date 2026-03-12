import { describe, it, expect, vi } from 'vitest'
import { smDefinitionAfterCreate } from './state-machines'
import type { HookContext } from './index'

function mockDb(data: Record<string, any[]> = {}) {
  return {
    findInCollection: vi.fn(async (collection: string, where: any) => {
      return { docs: data[collection] || [], totalDocs: 0 }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      return { id: `new-${collection}-${body.name || body.title || 'id'}`, ...body }
    }),
  }
}

describe('smDefinitionAfterCreate', () => {
  it('creates statuses, event types, and transitions from transition data', async () => {
    const db = mockDb({
      nouns: [{ id: 'noun-sr', name: 'SupportRequest' }],
    })
    const doc = {
      id: 'smd1',
      title: 'SupportRequest',
      domain: 'd1',
      transitions: [
        { from: 'Received', to: 'Triaging', event: 'acknowledge' },
        { from: 'Triaging', to: 'Investigating', event: 'assign' },
      ],
    }
    const ctx: HookContext = {
      domainId: 'd1',
      allNouns: [{ name: 'SupportRequest', id: 'noun-sr' }],
    }

    const result = await smDefinitionAfterCreate(db, doc, ctx)

    // Should create 3 statuses (Received, Triaging, Investigating)
    const statusCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'statuses'
    )
    expect(statusCreates.length).toBe(3)

    // Should create 2 event types (acknowledge, assign)
    const eventCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'event-types'
    )
    expect(eventCreates.length).toBe(2)

    // Should create 2 transitions
    const transitionCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'transitions'
    )
    expect(transitionCreates.length).toBe(2)
  })

  it('does nothing when no transitions provided', async () => {
    const db = mockDb()
    const doc = { id: 'smd1', title: 'Empty', domain: 'd1' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }
    const result = await smDefinitionAfterCreate(db, doc, ctx)
    expect(db.createInCollection).not.toHaveBeenCalled()
    expect(result.warnings).toHaveLength(0)
  })

  it('creates guards when guard text is provided', async () => {
    const db = mockDb()
    const doc = {
      id: 'smd1',
      title: 'Order',
      domain: 'd1',
      transitions: [
        { from: 'Pending', to: 'Shipped', event: 'ship', guard: 'paymentConfirmed' },
      ],
    }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }
    const result = await smDefinitionAfterCreate(db, doc, ctx)

    const guardCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'guards'
    )
    expect(guardCreates.length).toBe(1)
    expect(guardCreates[0][1].name).toBe('paymentConfirmed')
  })
})
