/**
 * arestAuthProvider tests.
 *
 * AREST is fact-based (whitepaper §3–§7): the user is an entity in P,
 * and `/arest/` returns its representation with `_links`. There is no
 * SPA-side auth logic — authentication is enforced upstream by the
 * `apis.` proxy, which stamps a session cookie on the browser before
 * requests reach the AREST worker.
 *
 * This provider is therefore a thin cookie-forwarding wrapper:
 *   - `login` / `logout` do NOT post to /arest/auth/*. They redirect
 *     the browser to the upstream login/logout URL if one was
 *     configured, or throw otherwise. The SPA never renders a login
 *     form.
 *   - `checkAuth` / `getIdentity` / `getPermissions` all probe
 *     `GET /arest/` (the HATEOAS root) with `credentials: include`
 *     and let the proxy's cookie do the work. 401 → caller-level
 *     redirect.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createArestAuthProvider } from '../arestAuthProvider'

interface Recorded {
  url: string
  method: string
  credentials?: RequestCredentials
  headers?: Record<string, string>
}

function stubFetch(responder: (req: Recorded) => Response): Recorded[] {
  const recorded: Recorded[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    const method = (init?.method ?? 'GET').toUpperCase()
    const headers: Record<string, string> = {}
    if (init?.headers) {
      const h = new Headers(init.headers as HeadersInit)
      h.forEach((v, k) => { headers[k] = v })
    }
    const req: Recorded = { url, method, credentials: init?.credentials, headers }
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

  afterEach(() => { vi.unstubAllGlobals() })

  describe('login (handled upstream)', () => {
    it('rejects with a clear message when no loginUrl is configured', async () => {
      const provider = createArestAuthProvider({ baseUrl })
      await expect(provider.login({ username: 'x', password: 'y' }))
        .rejects.toThrow(/upstream|edge|apis/i)
    })

    it('redirects to the configured loginUrl and never calls /arest/auth/login', async () => {
      const recorded = stubFetch(() => json({ ok: true }))
      const href = vi.fn()
      const provider = createArestAuthProvider({
        baseUrl,
        loginUrl: 'https://apis.auto.dev/login?redirect=/',
        navigate: href,
      })
      // login() hands off to the navigator; its promise may never resolve,
      // so we race it against a microtask deadline.
      const race = Promise.race([
        provider.login({}),
        new Promise((resolve) => setTimeout(resolve, 10)),
      ])
      await race
      expect(href).toHaveBeenCalledWith('https://apis.auto.dev/login?redirect=/')
      // Critically: the provider never posts anywhere.
      expect(recorded).toEqual([])
    })
  })

  describe('logout (handled upstream)', () => {
    it('resolves as a no-op when no logoutUrl is configured (cookie is edge-managed)', async () => {
      const recorded = stubFetch(() => json({ ok: true }))
      const provider = createArestAuthProvider({ baseUrl })
      await expect(provider.logout()).resolves.toBeUndefined()
      expect(recorded).toEqual([])
    })

    it('redirects to the configured logoutUrl and never calls /arest/auth/logout', async () => {
      const recorded = stubFetch(() => json({ ok: true }))
      const href = vi.fn()
      const provider = createArestAuthProvider({
        baseUrl,
        logoutUrl: 'https://apis.auto.dev/logout',
        navigate: href,
      })
      const race = Promise.race([
        provider.logout(),
        new Promise((resolve) => setTimeout(resolve, 10)),
      ])
      await race
      expect(href).toHaveBeenCalledWith('https://apis.auto.dev/logout')
      expect(recorded).toEqual([])
    })
  })

  describe('checkAuth', () => {
    let provider: ReturnType<typeof createArestAuthProvider>
    beforeEach(() => { provider = createArestAuthProvider({ baseUrl }) })

    it('GETs /arest/ with credentials: include and resolves on 200', async () => {
      const recorded = stubFetch(() => json({
        type: 'User',
        data: { email: 'sam@driv.ly' },
        _links: {},
      }))
      await expect(provider.checkAuth()).resolves.toBeUndefined()
      expect(recorded[0].url).toBe('https://ui.auto.dev/arest/')
      expect(recorded[0].method).toBe('GET')
      expect(recorded[0].credentials).toBe('include')
    })

    it('rejects on 401 so callers can redirect to host-level login', async () => {
      stubFetch(() => json({ error: 'no session' }, 401))
      await expect(provider.checkAuth()).rejects.toThrow()
    })

    it('does NOT send an explicit Authorization header', async () => {
      const recorded = stubFetch(() => json({ data: { email: 'x' }, _links: {} }))
      await provider.checkAuth()
      // Auth is cookie-only — the provider must not attempt token logic.
      expect(recorded[0].headers?.authorization).toBeUndefined()
    })
  })

  describe('checkError', () => {
    let provider: ReturnType<typeof createArestAuthProvider>
    beforeEach(() => { provider = createArestAuthProvider({ baseUrl }) })

    it('rejects on 401', async () => {
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
    let provider: ReturnType<typeof createArestAuthProvider>
    beforeEach(() => { provider = createArestAuthProvider({ baseUrl }) })

    it('returns envelope data from /arest/; id falls back to email', async () => {
      stubFetch(() => json({
        type: 'User',
        data: { email: 'sam@driv.ly' },
        _links: {},
      }))
      const identity = await provider.getIdentity()
      expect(identity.email).toBe('sam@driv.ly')
      // /arest/ root resource is keyed by email; id falls back to email.
      expect(identity.id).toBe('sam@driv.ly')
    })

    it('rejects when /arest/ returns 401', async () => {
      stubFetch(() => json({ error: 'unauthenticated' }, 401))
      await expect(provider.getIdentity()).rejects.toThrow()
    })
  })

  describe('getPermissions', () => {
    let provider: ReturnType<typeof createArestAuthProvider>
    beforeEach(() => { provider = createArestAuthProvider({ baseUrl }) })

    it('flattens _links.organizations into factType:id permission strings', async () => {
      // /arest/ exposes _links.organizations — HATEOAS Theorem 4 navigation
      // links — annotated with the fact type that grants membership.
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

    it('returns an empty array on failure rather than throwing', async () => {
      stubFetch(() => json({}, 500))
      await expect(provider.getPermissions()).resolves.toEqual([])
    })
  })
})
