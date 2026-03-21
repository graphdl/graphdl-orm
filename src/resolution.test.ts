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

describe('resolveNounInChain', () => {
  it('finds noun in first (app) registry and returns it', async () => {
    const app = mockRegistry({ User: { domainSlug: 'app-users', domainDoId: 'do-app-1' } })
    const org = mockRegistry({})
    const global = mockRegistry({})

    const result = await resolveNounInChain('User', [app, org, global])

    expect(result).toEqual({ domainSlug: 'app-users', domainDoId: 'do-app-1', registryIndex: 0 })
  })

  it('falls through to second (org) registry when first returns null', async () => {
    const app = mockRegistry({})
    const org = mockRegistry({ Invoice: { domainSlug: 'org-billing', domainDoId: 'do-org-2' } })
    const global = mockRegistry({})

    const result = await resolveNounInChain('Invoice', [app, org, global])

    expect(result).toEqual({ domainSlug: 'org-billing', domainDoId: 'do-org-2', registryIndex: 1 })
  })

  it('falls through to third (global) registry', async () => {
    const app = mockRegistry({})
    const org = mockRegistry({})
    const global = mockRegistry({ Currency: { domainSlug: 'global-finance', domainDoId: 'do-global-3' } })

    const result = await resolveNounInChain('Currency', [app, org, global])

    expect(result).toEqual({ domainSlug: 'global-finance', domainDoId: 'do-global-3', registryIndex: 2 })
  })

  it('returns null when noun not found anywhere', async () => {
    const app = mockRegistry({})
    const org = mockRegistry({})
    const global = mockRegistry({})

    const result = await resolveNounInChain('NonExistent', [app, org, global])

    expect(result).toBeNull()
  })

  it('short-circuits: does not call later registries when first finds the noun', async () => {
    const app = mockRegistry({ User: { domainSlug: 'app-users', domainDoId: 'do-app-1' } })
    const org = mockRegistry({ User: { domainSlug: 'org-users', domainDoId: 'do-org-1' } })
    const global = mockRegistry({ User: { domainSlug: 'global-users', domainDoId: 'do-global-1' } })

    await resolveNounInChain('User', [app, org, global])

    expect(app.resolveNoun).toHaveBeenCalledWith('User')
    expect(org.resolveNoun).not.toHaveBeenCalled()
    expect(global.resolveNoun).not.toHaveBeenCalled()
  })
})
