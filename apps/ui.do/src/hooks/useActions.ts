/**
 * useActions(noun, id) — SM transition affordances for an entity.
 *
 * Fetches the entity's available transitions from
 *   GET /arest/{slug}/{id}/actions
 * (per the #148 OpenAPI introspection route), slugifying the noun
 * via the same `nounToSlug` convention the worker exposes so URLs
 * round-trip with the server's /arest/ surface.
 *
 * Dispatching a named transition POSTs to
 *   POST /arest/{slug}/{id}/{action-name}
 * which the /arest/ collection handler routes to the underlying
 * `transition:<Noun>` system call (see src/api/router.ts:600).
 *
 * On a successful dispatch the hook invalidates three families of
 * TanStack Query keys — the same families the SSE bridge invalidates
 * on broadcast — so the UI refreshes optimistically without waiting
 * for the event round-trip:
 *   ['arest', 'list',      <slug>]
 *   ['arest', 'one',       <slug>, <id>]
 *   ['arest', 'reference', <slug>]
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { nounToSlug } from '../query'

export interface ArestAction {
  /** Event name as it appears in the state machine (e.g. "place"). */
  name: string
  /** Target status for the transition (e.g. "Placed"). */
  to: string
  /** Human-readable label. Falls back to Title-cased event name. */
  label: string
}

export interface UseActionsResult {
  actions: ArestAction[]
  dispatch: (actionName: string) => Promise<void>
  isLoading: boolean
}

export interface UseActionsOptions {
  /** e.g. 'https://ui.auto.dev/arest'. Required. */
  baseUrl: string
  /** Optional fetch override. Defaults to globalThis.fetch. */
  fetch?: typeof globalThis.fetch
}

interface RawTransition {
  event?: string
  name?: string
  targetStatus?: string
  to?: string
  label?: string
}

function titleCase(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1)
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}

/**
 * Pull the transitions array out of whatever shape the worker returned.
 * Accepts:
 *   [{event, targetStatus}, ...]                         (bare)
 *   { data: { transitions: [...] } }                     (Thm-5 envelope)
 *   { data: [...] }                                      (envelope, array data)
 *   { transitions: [...] }                               (half-unwrapped)
 */
function extractTransitions(body: unknown): RawTransition[] {
  if (Array.isArray(body)) return body as RawTransition[]
  if (!isRecord(body)) return []
  if (Array.isArray(body.transitions)) return body.transitions as RawTransition[]
  const data = body.data
  if (Array.isArray(data)) return data as RawTransition[]
  if (isRecord(data) && Array.isArray(data.transitions)) {
    return data.transitions as RawTransition[]
  }
  return []
}

function normalizeTransitions(raw: RawTransition[]): ArestAction[] {
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
  const violations = body.violations
  if (Array.isArray(violations) && violations.length > 0) {
    const first = violations[0]
    if (isRecord(first)) {
      return (first.detail as string) || (first.reading as string) || fallback
    }
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

export function useActions(
  noun: string,
  id: string,
  options: UseActionsOptions,
): UseActionsResult {
  const baseUrl = options.baseUrl.replace(/\/$/, '')
  const fetchImpl = options.fetch ?? ((...args) => globalThis.fetch(...args))
  const resource = nounToSlug(noun)
  const encodedId = encodeURIComponent(id)
  const queryClient = useQueryClient()

  const query = useQuery({
    queryKey: ['arest', 'actions', resource, id],
    queryFn: async () => {
      const body = await request(
        `${baseUrl}/${resource}/${encodedId}/actions`,
        { method: 'GET' },
        fetchImpl,
      )
      return normalizeTransitions(extractTransitions(body))
    },
  })

  const mutation = useMutation({
    mutationFn: async (actionName: string) => {
      const body = await request(
        `${baseUrl}/${resource}/${encodedId}/${encodeURIComponent(actionName)}`,
        { method: 'POST' },
        fetchImpl,
      )
      return body
    },
    onSuccess: async () => {
      // Same invalidation family as the SSE bridge so the local dispatch
      // races ahead of the broadcast round-trip.
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['arest', 'list', resource] }),
        queryClient.invalidateQueries({ queryKey: ['arest', 'one', resource, id] }),
        queryClient.invalidateQueries({ queryKey: ['arest', 'reference', resource] }),
        queryClient.invalidateQueries({ queryKey: ['arest', 'actions', resource, id] }),
      ])
    },
  })

  return {
    actions: query.data ?? [],
    isLoading: query.isLoading,
    dispatch: async (actionName: string) => {
      await mutation.mutateAsync(actionName)
    },
  }
}
