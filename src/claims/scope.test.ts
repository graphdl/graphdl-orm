import { describe, it, expect } from 'vitest'
import { createScope, addNoun, resolveNoun, addSchema, resolveSchema } from './scope'

describe('createScope', () => {
  it('creates an empty scope', () => {
    const scope = createScope()
    expect(scope.nouns.size).toBe(0)
    expect(scope.schemas.size).toBe(0)
    expect(scope.skipped).toBe(0)
    expect(scope.errors).toEqual([])
  })
})

describe('resolveNoun', () => {
  it('resolves noun from local domain first', () => {
    const scope = createScope()
    addNoun(scope, { id: 'n1', name: 'Status', domainId: 'd1' })
    addNoun(scope, { id: 'n2', name: 'Status', domainId: 'd2' })

    const result = resolveNoun(scope, 'Status', 'd1')
    expect(result).toBeDefined()
    expect(result!.id).toBe('n1')
  })

  it('falls back to other domains in scope', () => {
    const scope = createScope()
    addNoun(scope, { id: 'n1', name: 'Status', domainId: 'd1' })

    const result = resolveNoun(scope, 'Status', 'd2')
    expect(result).toBeDefined()
    expect(result!.id).toBe('n1')
  })

  it('returns null for unresolvable noun', () => {
    const scope = createScope()
    const result = resolveNoun(scope, 'Missing', 'd1')
    expect(result).toBeNull()
  })
})

describe('resolveSchema', () => {
  it('resolves schema by reading text', () => {
    const scope = createScope()
    addSchema(scope, 'Customer has Name', { id: 'gs1' })

    const result = resolveSchema(scope, 'Customer has Name')
    expect(result).toBeDefined()
    expect(result!.id).toBe('gs1')
  })

  it('returns null for missing schema', () => {
    const scope = createScope()
    const result = resolveSchema(scope, 'Missing reading')
    expect(result).toBeNull()
  })
})
