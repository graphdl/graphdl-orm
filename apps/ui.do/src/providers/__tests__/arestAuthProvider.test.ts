/**
 * arestAuthProvider tests.
 *
 * AREST auth is session-cookie based (per #131). The provider's job is
 * to shape fetch calls so the worker stamps / revokes / observes the
 * cookie, and to translate HTTP errors into the react-admin-style auth
 * contract (`checkAuth` rejects when unauthed; `checkError` rejects
 * the request for 401/403).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createArestAuthProvider } from '../arestAuthProvider'

interface Recorded {
  url: string
  method: string
  credentials?: RequestCredentials
  body?: unknown
}

function stubFetch(responder: (req: Recorded) => Response): Recorded[] {
  const recorded: Recorded[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    const method = (init?.method ?? 'GET').toUpperCase()
    let body: unknown
    if (init?.body != null) {
      try { body = JSON.parse(init.body as string) } catch { body = init.body }
    }
    const req: Recorded = { url, method, credentials: init?.credentials, body }
    recorded.push(req)
    return responder(req)
  })
  return recorded
}

function json(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

describe('arestAuthProvider', () => {
  const baseUrl = 'https://ui.auto.dev/arest'
  let provider: ReturnType<typeof createArestAuthProvider>

  beforeEach(() => { provider = createArestAuthProvider({ baseUrl }) })
  afterEach(() => { vi.unstubAllGlobals() })

  describe('login', () => {
    it('POSTs credentials to /arest/auth/login with credentials: include', async () => {
      const recorded = stubFetch(() => json({ ok: true }))

      await provider.login({ username: 'sam@driv.ly', password: 'sekret' })

      expect(recorded).toHaveLength(1)
      expect(recorded[0].url).toBe('https://ui.auto.dev/arest/auth/login')
      expect(recorded[0].method).toBe('POST')
      expect(recorded[0].credentials).toBe('include')
      expect(recorded[0].body).toEqual({ username: 'sam@driv.ly', password: 'sekret' })
    })

    it('rejects when the worker returns 401', async () => {
      stubFetch(() => json({ error: 'Bad credentials' }, 401))
      await expect(provider.login({ username: 'x', password: 'y' }))
        .rejects.toThrow(/Bad credentials|401/)
    })
  })

  describe('logout', () => {
    it('POSTs /arest/auth/logout with credentials: include', async () => {
      const recorded = stubFetch(() => json({ ok: true }))
      await provider.logout()
      expect(recorded[0].url).toBe('https://ui.auto.dev/arest/auth/logout')
      expect(recorded[0].method).toBe('POST')
      expect(recorded[0].credentials).toBe('include')
    })

    it('resolves even when the worker returns 401 (already logged out)', async () => {
      stubFetch(() => json({ error: 'no session' }, 401))
      await expect(provider.logout()).resolves.toBeUndefined()
    })
  })

  describe('checkAuth', () => {
    it('resolves when /arest/auth/me returns 200', async () => {
      const recorded = stubFetch(() => json({ data: { email: 'sam@driv.ly' }, _links: {} }))
      await expect(provider.checkAuth()).resolves.toBeUndefined()
      expect(recorded[0].url).toBe('https://ui.auto.dev/arest/auth/me')
      expect(recorded[0].credentials).toBe('include')
    })

    it('rejects when /arest/auth/me returns 401', async () => {
      stubFetch(() => json({ error: 'not signed in' }, 401))
      await expect(provider.checkAuth()).rejects.toThrow()
    })
  })

  describe('checkError', () => {
    it('rejects on 401', async () => {
      stubFetch(() => json({}, 200))
      await expect(provider.checkError({ status: 401 } as unknown as Error)).rejects.toThrow()
    })

    it('rejects on 403', async () => {
      await expect(provider.checkError({ status: 403 } as unknown as Error)).rejects.toThrow()
    })

    it('passes through other errors', async () => {
      await expect(provider.checkError({ status: 500 } as unknown as Error)).resolves.toBeUndefined()
      await expect(provider.checkError(new Error('network'))).resolves.toBeUndefined()
    })
  })

  describe('getIdentity', () => {
    it('returns the envelope data unwrapped with id=email fallback', async () => {
      stubFetch(() => json({
        data: { email: 'sam@driv.ly' },
        _links: {},
      }))
      const identity = await provider.getIdentity()
      expect(identity.email).toBe('sam@driv.ly')
      // AREST's /arest/ root resource is keyed by email, so id falls
      // back to email when the body doesn't carry a separate id.
      expect(identity.id).toBe('sam@driv.ly')
    })

    it('rejects when no identity is returned', async () => {
      stubFetch(() => json({ error: 'unauthenticated' }, 401))
      await expect(provider.getIdentity()).rejects.toThrow()
    })
  })

  describe('getPermissions', () => {
    it('returns a flat array of org-membership fact-type names', async () => {
      // The /arest/ root returns _links.organizations which encodes the
      // fact-type (User_owns_* / _administers_ / _belongs_to_). The auth
      // provider flattens those to a simple string[] suitable for
      // role-based UI gating.
      stubFetch(() => json({
        type: 'User',
        id: 'sam@driv.ly',
        data: { email: 'sam@driv.ly' },
        _links: {
          self: { href: '/arest/' },
          organizations: [
            { href: '/arest/organizations/acme', title: 'Acme', factType: 'User_owns_Organization' },
            { href: '/arest/organizations/globex', title: 'Globex', factType: 'User_belongs_to_Organization' },
          ],
        },
      }))
      const perms = await provider.getPermissions()
      expect(perms).toContain('User_owns_Organization:acme')
      expect(perms).toContain('User_belongs_to_Organization:globex')
    })

    it('returns an empty array on failure', async () => {
      stubFetch(() => json({}, 500))
      await expect(provider.getPermissions()).resolves.toEqual([])
    })
  })
})
