/**
 * arestAuthProvider — thin cookie-forwarding wrapper. Authentication
 * is enforced upstream by the `apis.` proxy; this provider does NOT
 * implement login / logout / token flows.
 *
 * Per whitepaper §3–§7 the user is an entity in P. `GET /arest/`
 * returns its representation with `_links` (HATEOAS, Theorem 4).
 *   checkAuth    → GET /arest/ (200 ⇒ signed in; 401 ⇒ redirect upstream)
 *   getIdentity  → GET /arest/, unwrap envelope
 *   getPermissions → GET /arest/, flatten _links.organizations
 *
 * login / logout are kept in the interface shape only because
 * @mdxui/app's AuthContext expects them. They either redirect the
 * browser to a configured upstream URL or reject with an explanatory
 * message — the SPA never renders a login form.
 */
import type {
  ArestAuthProvider,
  LoginParams,
  UserIdentity,
} from './types'

export interface ArestAuthProviderOptions {
  /** e.g. 'https://ui.auto.dev/arest' */
  baseUrl: string
  /**
   * Where to send the browser when login is needed. If absent, `login()`
   * rejects — useful for tests / SSR. In production this always points
   * at the upstream `apis.` host.
   */
  loginUrl?: string
  /** Where to send the browser on logout. If absent, `logout()` is a no-op. */
  logoutUrl?: string
  /**
   * Navigator used to hand off to the upstream flow. Defaults to
   *   (href) => { window.location.href = href }
   * Injectable for tests.
   */
  navigate?: (href: string) => void
  fetch?: typeof globalThis.fetch
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}

function defaultNavigate(href: string): void {
  if (typeof window !== 'undefined') {
    window.location.href = href
  }
}

async function jsonFetch(
  url: string,
  init: RequestInit,
  fetchImpl: typeof globalThis.fetch,
): Promise<{ ok: boolean; status: number; body: unknown }> {
  const res = await fetchImpl(url, {
    credentials: 'include',
    ...init,
    headers: {
      accept: 'application/json',
      ...(init.body ? { 'content-type': 'application/json' } : {}),
      ...(init.headers ?? {}),
    },
  })
  const text = await res.text()
  let body: unknown = null
  if (text) {
    try { body = JSON.parse(text) } catch { body = text }
  }
  return { ok: res.ok, status: res.status, body }
}

export function createArestAuthProvider(
  options: ArestAuthProviderOptions,
): ArestAuthProvider {
  const baseUrl = options.baseUrl.replace(/\/$/, '')
  const fetchImpl = options.fetch ?? ((...args) => globalThis.fetch(...args))
  const navigate = options.navigate ?? defaultNavigate

  // Single endpoint for every identity probe — the HATEOAS root.
  const rootUrl = `${baseUrl}/`

  const login = async (_params: LoginParams): Promise<void> => {
    if (options.loginUrl) {
      navigate(options.loginUrl)
      // Never resolve — the browser is navigating away. Holding the
      // promise open prevents the caller from racing past the hand-off.
      return new Promise(() => {})
    }
    throw new Error(
      'Login is handled upstream by the apis proxy. Configure loginUrl ' +
      'on the auth provider to redirect at the edge instead of rendering ' +
      'a login form in the SPA.',
    )
  }

  const logout = async (): Promise<void> => {
    if (options.logoutUrl) {
      navigate(options.logoutUrl)
      return new Promise(() => {})
    }
    // Session cookie is scoped to the edge — nothing for the SPA to do.
  }

  const checkAuth = async (): Promise<void> => {
    const res = await jsonFetch(rootUrl, { method: 'GET' }, fetchImpl)
    if (!res.ok) {
      throw new Error(
        isRecord(res.body) && typeof res.body.error === 'string'
          ? (res.body.error as string)
          : `not authenticated (HTTP ${res.status})`,
      )
    }
  }

  const checkError = async (error: Error | { status?: number; message?: string }): Promise<void> => {
    const status = isRecord(error) ? (error.status as number | undefined) : undefined
    if (status === 401 || status === 403) {
      const msg = isRecord(error) ? (error.message as string | undefined) : undefined
      throw new Error(msg || `auth error (HTTP ${status})`)
    }
  }

  const getIdentity = async (): Promise<UserIdentity> => {
    const res = await jsonFetch(rootUrl, { method: 'GET' }, fetchImpl)
    if (!res.ok) throw new Error(`unauthenticated (HTTP ${res.status})`)

    const body = res.body
    const payload = isRecord(body) && isRecord(body.data) ? body.data : body
    if (!isRecord(payload)) throw new Error('unauthenticated')

    const email = (payload.email as string | undefined) ?? undefined
    // /arest/ root may surface id at the envelope level (e.g. `type: 'User',
    // id: 'sam@driv.ly'`) rather than on the inner `data`. Check both.
    const envelopeId = isRecord(body) ? (body.id as string | undefined) : undefined
    const id = (payload.id as string | undefined) ?? envelopeId ?? email
    if (!id) throw new Error('unauthenticated')

    const identity: UserIdentity = { id }
    if (email) identity.email = email
    if (typeof payload.fullName === 'string') identity.fullName = payload.fullName as string
    if (typeof payload.avatar === 'string') identity.avatar = payload.avatar as string

    for (const [key, value] of Object.entries(payload)) {
      if (key === 'id' || key === 'email' || key === 'fullName' || key === 'avatar') continue
      identity[key] = value
    }
    return identity
  }

  const getPermissions = async (): Promise<string[]> => {
    try {
      const res = await jsonFetch(rootUrl, { method: 'GET' }, fetchImpl)
      if (!res.ok) return []
      const body = res.body
      if (!isRecord(body)) return []
      const links = isRecord(body._links) ? body._links : {}
      const orgs = Array.isArray((links as Record<string, unknown>).organizations)
        ? ((links as Record<string, unknown>).organizations as unknown[])
        : []

      const permissions: string[] = []
      for (const entry of orgs) {
        if (!isRecord(entry)) continue
        const factType = entry.factType as string | undefined
        const href = entry.href as string | undefined
        if (!factType || !href) continue
        // href: /arest/organizations/{id}. Last segment is the id.
        const parts = href.split('/').filter(Boolean)
        const id = parts[parts.length - 1]
        permissions.push(`${factType}:${id}`)
      }
      return permissions
    } catch {
      return []
    }
  }

  return {
    login,
    logout,
    checkAuth,
    checkError,
    getIdentity,
    getPermissions,
  }
}
