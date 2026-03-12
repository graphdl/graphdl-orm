import { describe, it, expect, vi } from 'vitest'
import { readingAfterCreate } from './readings'
import type { HookContext } from './index'

function mockDb(data: Record<string, any[]> = {}) {
  return {
    findInCollection: vi.fn(async (collection: string, where: any) => {
      return { docs: data[collection] || [], totalDocs: 0 }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      return { id: `new-${collection}-${body.name || body.text || 'id'}`, ...body }
    }),
    updateInCollection: vi.fn(async () => ({})),
  }
}

describe('readingAfterCreate', () => {
  it('creates nouns, graph schema, and roles for a simple reading', async () => {
    const db = mockDb()
    const doc = { id: 'r1', text: 'Customer has Name.', domain: 'd1' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }

    const result = await readingAfterCreate(db, doc, ctx)

    // Should have created 2 nouns (Customer as entity, Name as value)
    const nounCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'nouns'
    )
    expect(nounCreates.length).toBe(2)

    // Should have created 1 graph schema
    const schemaCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'graph-schemas'
    )
    expect(schemaCreates.length).toBe(1)
    expect(schemaCreates[0][1].name).toBe('CustomerName')

    // Should have created 2 roles
    const roleCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'roles'
    )
    expect(roleCreates.length).toBe(2)
  })

  it('delegates indented constraint lines via createWithHook', async () => {
    const db = mockDb()
    const doc = {
      id: 'r1',
      text: 'Customer has Name.\n  Each Customer has at most one Name.',
      domain: 'd1',
    }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }

    const result = await readingAfterCreate(db, doc, ctx)

    // Should have created a constraint (delegated)
    const constraintCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'constraints'
    )
    expect(constraintCreates.length).toBeGreaterThanOrEqual(1)
  })

  it('reuses existing nouns (idempotency)', async () => {
    const db = mockDb({
      nouns: [
        { id: 'existing-customer', name: 'Customer' },
        { id: 'existing-name', name: 'Name' },
      ],
    })
    const doc = { id: 'r1', text: 'Customer has Name.', domain: 'd1' }
    const ctx: HookContext = {
      domainId: 'd1',
      allNouns: [
        { name: 'Customer', id: 'existing-customer' },
        { name: 'Name', id: 'existing-name' },
      ],
    }

    await readingAfterCreate(db, doc, ctx)

    // Should NOT have created new nouns
    const nounCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'nouns'
    )
    expect(nounCreates.length).toBe(0)
  })
})
