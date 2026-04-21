/**
 * useActions(noun, id) — state-machine transition affordances for an
 * entity.
 *
 * Wires the worker's entity transition endpoints
 * (src/api/router.ts:573–677):
 *
 *   GET  /api/entities/{noun}/{id}/transitions
 *     -> { currentStatus, transitions: [{ event, targetStatus }] }
 *
 *   POST /api/entities/{noun}/{id}/transition
 *     body: { event, domain? }
 *     -> { id, noun, previousStatus, status, event, transitions }
 *
 * `noun` is the human-readable noun name (e.g. "Support Request").
 * It's URL-encoded (so "Support Request" -> "Support%20Request") to
 * match the router's :noun parameter, which decodes the same way
 * server-side. We still compute the plural slug so the hook can
 * invalidate the same TanStack Query keys the SSE bridge uses.
 *
 * On a successful dispatch the hook invalidates:
 *   ['arest', 'list',      <slug>]            — prefix match
 *   ['arest', 'one',       <slug>, <id>]      — exact
 *   ['arest', 'reference', <slug>]            — prefix match
 *   ['arest', 'actions',   <slug>, <id>]      — this hook's own key
 * — matching the SSE bridge's key family (see arestQueryBridge.ts)
 * so a local dispatch races ahead of the broadcast without the UI
 * double-refetching when the event arrives.
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { nounToSlug } from '../query'

export interface ArestAction {
  /** Event name (e.g. "place"). */
  name: string
  /** Target status (e.g. "Placed"). Empty string if the response omits it. */
  to: string
  /** Display label — falls back to Title(name). */
  label: string
}

export interface UseActionsResult {
  actions: ArestAction[]
  currentStatus: string | null
  dispatch: (actionName: string) => Promise<void>
  isLoading: boolean
}

export interface UseActionsOptions {
  /**
   * Worker host. Must include `/arest` because we use the same base
   * URL as the data provider — the hook strips the `/arest` suffix
   * to reach the sibling `/api/entities` route.
   */
  baseUrl: string
  /** Optional domain filter forwarded to POST body. */
  domain?: string
  /** Optional fetch override. Defaults to globalThis.fetch. */
  fetch?: typeof globalThis.fetch
}

interface TransitionResponse {
  currentStatus?: string
  transitions?: Array<{ event?: string; name?: string; targetStatus?: string; to?: string; label?: string }>
  // Also tolerate the envelope shape some callers use:
  data?: {
    transitions?: Array<{ event?: string; targetStatus?: string; label?: string }>
    status?: string
    currentStatus?: string
  }
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}

function titleCase(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1)
}

function extractTransitions(body: unknown): Array<{ event?: string; name?: string; targetStatus?: string; to?: string; label?: string }> {
  if (Array.isArray(body)) return body
  if (!isRecord(body)) return []
  const b = body as TransitionResponse
  if (Array.isArray(b.transitions)) return b.transitions
  if (isRecord(b.data) && Array.isArray(b.data.transitions)) return b.data.transitions
  return []
}

function extractCurrentStatus(body: unknown): string | null {
  if (!isRecord(body)) return null
  const b = body as TransitionResponse
  if (typeof b.currentStatus === 'string') return b.currentStatus
  if (isRecord(b.data)) {
    if (typeof b.data.currentStatus === 'string') return b.data.currentStatus
    if (typeof b.data.status === 'string') return b.data.status
  }
  return null
}

function normalizeTransitions(raw: Array<{ event?: string; name?: string; targetStatus?: string; to?: string; label?: string }>): ArestAction[] {
  return raw
    .map((t) => {
      const name = t.event ?? t.name ?? ''
      if (!name) return null
      const to = t.targetStatus ?? t.to ?? ''
      const label = t.label ?? titleCase(name)
      return { name, to, label } as ArestAction
    })
    .filter((a): a is ArestAction => a !== null)
}

function violationMessage(body: unknown, fallback: string): string {
  if (!isRecord(body)) return fallback
  const violations = body.violations ?? (isRecord(body.errors) ? [body.errors] : body.errors)
  if (Array.isArray(violations) && violations.length > 0) {
    const first = violations[0]
    if (isRecord(first)) {
      return (first.detail as string) || (first.reading as string) || (first.message as string) || fallback
    }
  }
  if (Array.isArray(body.errors) && body.errors.length > 0) {
    const first = body.errors[0]
    if (isRecord(first) && typeof first.message === 'string') return first.message as string
  }
  return fallback
}

async function request(
  url: string,
  init: RequestInit,
  fetchImpl: typeof globalThis.fetch,
): Promise<unknown> {
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
  if (!res.ok) {
    throw new Error(violationMessage(body, `HTTP ${res.status}`))
  }
  return body
}

/**
 * Derive the worker root (without `/arest`) so we can reach the
 * sibling `/api/entities/...` routes. Matches the same trick
 * arestNavigationProvider uses for `/api/openapi.json`.
 */
function workerRoot(baseUrl: string): string {
  const trimmed = baseUrl.replace(/\/$/, '')
  return trimmed.endsWith('/arest') ? trimmed.slice(0, -'/arest'.length) : trimmed
}

export function useActions(
  noun: string,
  id: string,
  options: UseActionsOptions,
): UseActionsResult {
  const fetchImpl = options.fetch ?? ((...args) => globalThis.fetch(...args))
  const root = workerRoot(options.baseUrl)
  const slug = nounToSlug(noun)
  const encodedNoun = encodeURIComponent(noun)
  const encodedId = encodeURIComponent(id)
  const queryClient = useQueryClient()

  const query = useQuery({
    queryKey: ['arest', 'actions', slug, id],
    queryFn: async () => {
      const body = await request(
        `${root}/api/entities/${encodedNoun}/${encodedId}/transitions`,
        { method: 'GET' },
        fetchImpl,
      )
      return {
        actions: normalizeTransitions(extractTransitions(body)),
        currentStatus: extractCurrentStatus(body),
      }
    },
  })

  const mutation = useMutation({
    mutationFn: async (actionName: string) => {
      return request(
        `${root}/api/entities/${encodedNoun}/${encodedId}/transition`,
        {
          method: 'POST',
          body: JSON.stringify({
            event: actionName,
            ...(options.domain ? { domain: options.domain } : {}),
          }),
        },
        fetchImpl,
      )
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['arest', 'list', slug] }),
        queryClient.invalidateQueries({ queryKey: ['arest', 'one', slug, id] }),
        queryClient.invalidateQueries({ queryKey: ['arest', 'reference', slug] }),
        queryClient.invalidateQueries({ queryKey: ['arest', 'actions', slug, id] }),
      ])
    },
  })

  return {
    actions: query.data?.actions ?? [],
    currentStatus: query.data?.currentStatus ?? null,
    isLoading: query.isLoading,
    dispatch: async (actionName: string) => {
      await mutation.mutateAsync(actionName)
    },
  }
}
