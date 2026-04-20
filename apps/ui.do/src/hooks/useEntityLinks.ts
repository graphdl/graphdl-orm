/**
 * useEntityLinks(noun, id) — HATEOAS navigation affordances for an
 * entity, extracted from the Theorem-4 `_links` section of the
 * response envelope.
 *
 * Theorem 4 (paper §7): for noun e in state s,
 *   links_full(e, n, s) = nav(e, n) ∪ links(s)
 * The server emits both halves under `_links`:
 *   _links.transitions  — state-machine events (consumed by useActions)
 *   _links.navigation   — navigation links, keyed by rel
 *   _links.<key>        — individual navigation entries (parent / child
 *                         collection / peer). These are standard
 *                         HAL-flavoured entries with href + optional
 *                         title + optional factType.
 *
 * `useEntityLinks` surfaces the navigation half only — transitions
 * belong to `useActions`. Consumers compose the two into an
 * OverworldMenu that shows both "actions you can take here" and
 * "places you can go from here".
 */
import { useQuery } from '@tanstack/react-query'
import { nounToSlug } from '../query'

export interface EntityNavLink {
  /** Relation name from _links (e.g. "organization", "domains"). */
  rel: string
  /** Target URL path. */
  href: string
  /** Human-readable label for menu rendering. */
  label: string
  /** Optional fact-type identifier the link was projected over. */
  factType?: string
}

export interface UseEntityLinksOptions {
  baseUrl: string
  fetch?: typeof globalThis.fetch
}

export interface UseEntityLinksResult {
  links: EntityNavLink[]
  isLoading: boolean
  error?: unknown
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}

function humanize(s: string): string {
  return s
    .replace(/[_-]/g, ' ')
    .split(/\s+/)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ')
}

/**
 * Walk a response body and normalise its `_links` block into a flat
 * list of nav entries. Skips `self`, `create`, and the transition
 * list (those belong to different surfaces).
 */
export function extractNavLinks(body: unknown): EntityNavLink[] {
  if (!isRecord(body)) return []
  const links = isRecord(body._links) ? body._links : null
  if (!links) return []

  const skipKeys = new Set(['self', 'create', 'transitions', 'navigation'])
  const out: EntityNavLink[] = []

  for (const [rel, entry] of Object.entries(links)) {
    if (skipKeys.has(rel)) continue

    // Single object form: { href, title?, factType? }
    if (isRecord(entry) && typeof entry.href === 'string') {
      out.push({
        rel,
        href: entry.href as string,
        label: (entry.title as string | undefined) ?? humanize(rel),
        factType: entry.factType as string | undefined,
      })
      continue
    }

    // Array form: [{ href, title?, factType? }, ...]
    if (Array.isArray(entry)) {
      for (const item of entry) {
        if (!isRecord(item) || typeof item.href !== 'string') continue
        out.push({
          rel,
          href: item.href as string,
          label: (item.title as string | undefined) ?? humanize(rel),
          factType: item.factType as string | undefined,
        })
      }
      continue
    }
  }

  // Theorem-4 also surfaces a `_links.navigation` block as a
  // flat href map for simpler consumers (see envelope.ts).
  if (isRecord(links.navigation)) {
    for (const [rel, href] of Object.entries(links.navigation)) {
      if (typeof href !== 'string') continue
      if (skipKeys.has(rel)) continue
      out.push({ rel, href, label: humanize(rel) })
    }
  }
  return out
}

export function useEntityLinks(
  noun: string,
  id: string,
  options: UseEntityLinksOptions,
): UseEntityLinksResult {
  const baseUrl = options.baseUrl.replace(/\/$/, '')
  const fetchImpl = options.fetch ?? ((...args) => globalThis.fetch(...args))
  const resource = nounToSlug(noun)
  const encodedId = encodeURIComponent(id)

  const query = useQuery({
    queryKey: ['arest', 'links', resource, id],
    queryFn: async () => {
      const res = await fetchImpl(`${baseUrl}/${resource}/${encodedId}`, {
        credentials: 'include',
        headers: { accept: 'application/json' },
      })
      if (!res.ok) throw new Error(`entity fetch failed (HTTP ${res.status})`)
      return res.json()
    },
  })

  return {
    links: extractNavLinks(query.data),
    isLoading: query.isLoading,
    error: query.error ?? undefined,
  }
}
