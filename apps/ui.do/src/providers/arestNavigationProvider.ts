/**
 * arestNavigationProvider — resource set + menu derived from the
 * worker's OpenAPI document (per #117).
 *
 * The worker exposes OpenAPI per App at GET /api/openapi.json?app=<name>.
 * Any top-level path `/arest/{slug}` (no further segments, no template
 * variables) is treated as a list collection and becomes an mdxui
 * resource. Menu items are the same set lifted to `{ title, url }`.
 *
 * Note: the provider hits /api/openapi.json, which lives on the
 * worker base URL's **sibling** of /arest (same origin, different
 * path prefix). We derive the openapi URL from baseUrl by stripping a
 * trailing /arest if present.
 */
import type {
  ArestMenuItem,
  ArestNavigationProvider,
  ArestResource,
} from './types'

export interface ArestNavigationProviderOptions {
  /** AREST worker base URL (e.g. https://ui.auto.dev/arest). */
  baseUrl: string
  /** OpenAPI document selector — maps to ?app=<name>. Defaults to 'ui.do'. */
  app?: string
  fetch?: typeof globalThis.fetch
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}

/** Uppercase the first letter and replace hyphens with spaces. */
function humanize(slug: string): string {
  return slug
    .split('-')
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ')
}

/** "support-requests" -> "support-request" (trailing-s drop). */
function singularize(slug: string): string {
  return slug.endsWith('s') ? slug.slice(0, -1) : slug
}

interface PathsDoc {
  paths?: Record<string, Record<string, unknown>>
}

function extractResources(doc: unknown): ArestResource[] {
  if (!isRecord(doc)) return []
  const pathsDoc = doc as PathsDoc
  const paths = pathsDoc.paths ?? {}
  const resources: ArestResource[] = []
  for (const [path, methods] of Object.entries(paths)) {
    if (!path.startsWith('/arest/')) continue
    // Must be exactly /arest/{slug} — no trailing segments, no {id} vars.
    const rest = path.slice('/arest/'.length)
    if (!rest || rest.includes('/') || rest.includes('{')) continue
    const slug = rest
    if (!slug) continue

    const getOp = isRecord(methods) ? (methods as Record<string, unknown>).get : undefined
    const singular = isRecord(getOp) && typeof (getOp as Record<string, unknown>)['x-singular'] === 'string'
      ? ((getOp as Record<string, unknown>)['x-singular'] as string)
      : humanize(singularize(slug))

    resources.push({
      name: slug,
      label: singular,
      labelPlural: humanize(slug),
    })
  }
  return resources
}

export function createArestNavigationProvider(
  options: ArestNavigationProviderOptions,
): ArestNavigationProvider {
  const baseUrl = options.baseUrl.replace(/\/$/, '')
  const app = options.app ?? 'ui.do'
  const fetchImpl = options.fetch ?? ((...args) => globalThis.fetch(...args))

  // OpenAPI lives at /api/openapi.json on the worker (sibling of /arest).
  // Strip trailing /arest from the configured baseUrl to find the worker root.
  const workerRoot = baseUrl.endsWith('/arest') ? baseUrl.slice(0, -'/arest'.length) : baseUrl
  const openapiUrl = `${workerRoot}/api/openapi.json?app=${encodeURIComponent(app)}`

  let cache: ArestResource[] | null = null

  const resources = async (): Promise<ArestResource[]> => {
    if (cache) return cache
    try {
      const response = await fetchImpl(openapiUrl, {
        credentials: 'include',
        headers: { accept: 'application/json' },
      })
      if (!response.ok) {
        cache = []
        return cache
      }
      const doc = await response.json()
      cache = extractResources(doc)
      return cache
    } catch {
      cache = []
      return cache
    }
  }

  const menu = async (): Promise<ArestMenuItem[]> => {
    const list = await resources()
    return list.map((r) => ({ title: r.labelPlural, url: `/${r.name}` }))
  }

  return { resources, menu }
}
