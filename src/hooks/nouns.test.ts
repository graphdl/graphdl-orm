import { describe, it, expect, vi } from 'vitest'
import { nounAfterCreate } from './nouns'
import type { HookContext } from './index'

function mockDb(data: Record<string, any[]> = {}) {
  return {
    findInCollection: vi.fn(async (collection: string, where: any) => {
      return { docs: data[collection] || [], totalDocs: 0 }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      return { id: `new-${collection}-id`, ...body }
    }),
    updateInCollection: vi.fn(async (collection: string, id: string, body: any) => {
      return { id, ...body }
    }),
  }
}

describe('nounAfterCreate', () => {
  it('does nothing for a noun without subtype text', async () => {
    const db = mockDb()
    const doc = { id: 'n1', name: 'Customer', objectType: 'entity', domain: 'd1' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }
    const result = await nounAfterCreate(db, doc, ctx)
    expect(result.warnings).toHaveLength(0)
    expect(db.createInCollection).not.toHaveBeenCalled()
  })

  it('parses subtype text and sets superType', async () => {
    const db = mockDb({
      nouns: [{ id: 'parent-id', name: 'Request', objectType: 'entity' }],
    })
    const doc = { id: 'n1', name: 'SupportRequest', objectType: 'entity', domain: 'd1',
      promptText: 'SupportRequest is a subtype of Request' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [{ name: 'Request', id: 'parent-id' }] }
    const result = await nounAfterCreate(db, doc, ctx)
    expect(db.updateInCollection).toHaveBeenCalledWith('nouns', 'n1', { superType: 'parent-id' })
    expect(result.warnings).toHaveLength(0)
  })

  it('creates parent noun if not found', async () => {
    const db = mockDb({ nouns: [] })
    const doc = { id: 'n1', name: 'SupportRequest', objectType: 'entity', domain: 'd1',
      promptText: 'SupportRequest is a subtype of Request' }
    const ctx: HookContext = { domainId: 'd1', allNouns: [] }
    const result = await nounAfterCreate(db, doc, ctx)
    expect(db.createInCollection).toHaveBeenCalledWith('nouns', expect.objectContaining({
      name: 'Request', objectType: 'entity', domain: 'd1',
    }))
    expect(result.created['nouns']).toHaveLength(1)
  })
})
