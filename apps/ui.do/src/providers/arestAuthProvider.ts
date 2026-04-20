/**
 * arestAuthProvider — session-cookie auth against the AREST worker
 * (per #131). Cookies are opaque to the browser; the provider only
 * adjusts fetch's `credentials: include` option so the browser sends
 * / receives them on each request.
 *
 * /arest/auth/login, /arest/auth/logout, /arest/auth/me are the three
 * endpoints used. /arest/ (root) is hit for permissions — its
 * _links.organizations carries the fact-type that grants membership,
 * which we flatten into a permissions array the UI can role-gate on.
 */
import type {
  ArestAuthProvider,
  LoginParams,
  UserIdentity,
} from './types'

export interface ArestAuthProviderOptions {
  baseUrl: string
  fetch?: typeof globalThis.fetch
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
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

function errorMessage(body: unknown, fallback: string): string {
  if (isRecord(body) && typeof body.error === 'string') return body.error
  if (isRecord(body) && Array.isArray(body.errors)) {
    const first = (body.errors as unknown[])[0]
    if (isRecord(first) && typeof first.message === 'string') return first.message as string
  }
  return fallback
}

export function createArestAuthProvider(
  options: ArestAuthProviderOptions,
): ArestAuthProvider {
  const baseUrl = options.baseUrl.replace(/\/$/, '')
  const fetchImpl = options.fetch ?? ((...args) => globalThis.fetch(...args))

  const login = async (params: LoginParams): Promise<void> => {
    const res = await jsonFetch(
      `${baseUrl}/auth/login`,
      { method: 'POST', body: JSON.stringify(params) },
      fetchImpl,
    )
    if (!res.ok) {
      throw new Error(errorMessage(res.body, `login failed (HTTP ${res.status})`))
    }
  }

  const logout = async (): Promise<void> => {
    // Best-effort. If the session is already gone the worker may return
    // 401 — that's still a successful logout from the client's view.
    await jsonFetch(`${baseUrl}/auth/logout`, { method: 'POST' }, fetchImpl)
  }

  const checkAuth = async (): Promise<void> => {
    const res = await jsonFetch(`${baseUrl}/auth/me`, { method: 'GET' }, fetchImpl)
    if (!res.ok) throw new Error(errorMessage(res.body, 'not authenticated'))
  }

  const checkError = async (error: Error | { status?: number; message?: string }): Promise<void> => {
    const status = isRecord(error) ? (error.status as number | undefined) : undefined
    if (status === 401 || status === 403) {
      const msg = isRecord(error) ? (error.message as string | undefined) : undefined
      throw new Error(msg || `auth error (HTTP ${status})`)
    }
  }

  const getIdentity = async (): Promise<UserIdentity> => {
    const res = await jsonFetch(`${baseUrl}/auth/me`, { method: 'GET' }, fetchImpl)
    if (!res.ok) throw new Error(errorMessage(res.body, 'unauthenticated'))

    const body = res.body
    const payload = isRecord(body) && isRecord(body.data) ? body.data : body
    if (!isRecord(payload)) throw new Error('unauthenticated')

    const email = (payload.email as string | undefined) ?? undefined
    const id = (payload.id as string | undefined) ?? email
    if (!id) throw new Error('unauthenticated')

    const identity: UserIdentity = { id }
    if (email) identity.email = email
    if (typeof payload.fullName === 'string') identity.fullName = payload.fullName as string
    if (typeof payload.avatar === 'string') identity.avatar = payload.avatar as string

    // Carry extra fields forward so UI can render richer profiles.
    for (const [key, value] of Object.entries(payload)) {
      if (key === 'id' || key === 'email' || key === 'fullName' || key === 'avatar') continue
      identity[key] = value
    }
    return identity
  }

  const getPermissions = async (): Promise<string[]> => {
    try {
      const res = await jsonFetch(`${baseUrl}/`, { method: 'GET' }, fetchImpl)
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
        // href: /arest/organizations/{id}. Strip prefix to recover {id}.
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
