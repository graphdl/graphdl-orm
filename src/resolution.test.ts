import { describe, it, expect, vi } from 'vitest'
import type { RegistryStub } from './resolution'
import { resolveNounInChain } from './resolution'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function mockRegistry(map: Record<string, { domainSlug: string; domainDoId: string }>): RegistryStub {
  return {
    resolveNoun: vi.fn(async (nounName: string) => map[nounName] ?? null),
  }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('two-tier resolution (org -> public)', () => {
  it('resolves noun in org registry first', async () => {
    const orgRegistry = mockRegistry({ Customer: { domainSlug: 'tickets', domainDoId: 'do-1' } })
    const publicRegistry = mockRegistry({})

    const result = await resolveNounInChain('Customer', [orgRegistry, publicRegistry])

    expect(result).toEqual({ domainSlug: 'tickets', domainDoId: 'do-1', registryIndex: 0 })
    expect(publicRegistry.resolveNoun).not.toHaveBeenCalled()
  })

  it('falls through to public if org has no match', async () => {
    const orgRegistry = mockRegistry({})
    const publicRegistry = mockRegistry({ Noun: { domainSlug: 'core', domainDoId: 'do-2' } })

    const result = await resolveNounInChain('Noun', [orgRegistry, publicRegistry])

    expect(result).toEqual({ domainSlug: 'core', domainDoId: 'do-2', registryIndex: 1 })
  })

  it('returns null when noun not found in either registry', async () => {
    const orgRegistry = mockRegistry({})
    const publicRegistry = mockRegistry({})

    const result = await resolveNounInChain('NonExistent', [orgRegistry, publicRegistry])

    expect(result).toBeNull()
  })

  it('short-circuits: does not call public registry when org finds the noun', async () => {
    const orgRegistry = mockRegistry({ User: { domainSlug: 'org-users', domainDoId: 'do-org-1' } })
    const publicRegistry = mockRegistry({ User: { domainSlug: 'public-users', domainDoId: 'do-public-1' } })

    await resolveNounInChain('User', [orgRegistry, publicRegistry])

    expect(orgRegistry.resolveNoun).toHaveBeenCalledWith('User')
    expect(publicRegistry.resolveNoun).not.toHaveBeenCalled()
  })
})
