import { describe, it, expect, vi } from 'vitest'
import { constraintAfterCreate } from './constraints'
import type { HookContext } from './index'

function mockDb(data: Record<string, any[]> = {}) {
  return {
    findInCollection: vi.fn(async (collection: string, where: any) => {
      return { docs: data[collection] || [], totalDocs: (data[collection] || []).length }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      return { id: `new-${collection}-id`, ...body }
    }),
    updateInCollection: vi.fn(async (collection: string, id: string, body: any) => {
      return { id, ...body }
    }),
  }
}

describe('constraintAfterCreate', () => {
  const baseContext: HookContext = {
    domainId: 'd1',
    allNouns: [
      { name: 'Customer', id: 'n-customer' },
      { name: 'Name', id: 'n-name' },
    ],
  }

  it('parses natural language UC and creates constraint spans', async () => {
    const db = mockDb({
      readings: [{ id: 'r1', text: 'Customer has Name', domain: 'd1' }],
      roles: [
        { id: 'role-0', readingId: 'r1', nounId: 'n-customer', roleIndex: 0 },
        { id: 'role-1', readingId: 'r1', nounId: 'n-name', roleIndex: 1 },
      ],
    })
    const doc = { id: 'c1', text: 'Each Customer has at most one Name.', domain: 'd1' }

    const result = await constraintAfterCreate(db, doc, baseContext)

    // Should update constraint with parsed kind/modality
    expect(db.updateInCollection).toHaveBeenCalledWith('constraints', 'c1',
      expect.objectContaining({ kind: 'UC', modality: 'Alethic' })
    )
    // Should create constraint spans
    const spanCreates = db.createInCollection.mock.calls.filter(
      ([c]: [string]) => c === 'constraint-spans'
    )
    expect(spanCreates.length).toBeGreaterThanOrEqual(1)
  })

  it('rejects constraint when host reading not found (non-batch)', async () => {
    const db = mockDb({ readings: [] })
    const doc = { id: 'c1', text: 'Each Foo has at most one Bar.', domain: 'd1' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }

    const result = await constraintAfterCreate(db, doc, ctx)
    expect(result.warnings.length).toBeGreaterThan(0)
    expect(result.warnings[0]).toContain('host reading not found')
  })

  it('handles shorthand multiplicity format', async () => {
    const db = mockDb({
      readings: [{ id: 'r1', text: 'Customer has Name', domain: 'd1' }],
      roles: [
        { id: 'role-0', readingId: 'r1', nounId: 'n-customer', roleIndex: 0 },
        { id: 'role-1', readingId: 'r1', nounId: 'n-name', roleIndex: 1 },
      ],
    })
    const doc = { id: 'c1', multiplicity: '*:1', reading: 'Customer has Name', domain: 'd1' }

    const result = await constraintAfterCreate(db, doc, baseContext)

    expect(db.updateInCollection).toHaveBeenCalledWith('constraints', 'c1',
      expect.objectContaining({ kind: 'UC' })
    )
  })

  it('defers in batch mode when host reading not found', async () => {
    const db = mockDb({ readings: [] })
    const doc = { id: 'c1', text: 'Each Baz has at most one Qux.', domain: 'd1' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [], batch: true, deferred: [] }

    const result = await constraintAfterCreate(db, doc, ctx)
    expect(result.warnings).toHaveLength(0) // no warning in batch mode
    expect(ctx.deferred!.length).toBe(1) // deferred instead
  })
})
