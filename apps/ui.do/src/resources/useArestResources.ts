/**
 * useArestResources(options) — auto-discover every noun in the
 * app-scoped OpenAPI document and produce a ResourceDefinition for
 * each. Lets callers drop a single block into their Admin tree
 * instead of enumerating nouns by hand:
 *
 *   const { resources, isLoading } = useArestResources({ baseUrl, app })
 *   return <Admin resources={resources} />
 *
 * Discovery rules:
 *   - `components.schemas.<Name>` → one ResourceDefinition per
 *     schema whose type is "object" (entity types). Value types
 *     (schemas.<Name>.type !== 'object') are skipped.
 *   - `noun` is the schema name; `slug` is the canonical
 *     nounToSlug result so URLs match the worker.
 *   - Future: consult `/arest/explain` for SM / constraint hints to
 *     decide which views make sense (entity-only vs read-only vs
 *     action-only). For now every noun gets all four views — the
 *     generic views themselves handle gracefully-missing data.
 */
import { useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { ResourceDefinition } from './resourceDefinition'
import { createArestResource, type CreateArestResourceOptions } from './createArestResource'

export interface UseArestResourcesOptions {
  baseUrl: string
  app?: string
  fetch?: typeof globalThis.fetch
  /** Optional noun-to-label overrides. */
  labels?: Record<string, string>
  /**
   * Optional noun-name filter — return true to keep the noun, false
   * to drop it. Used by the hostname-based branding surface (#128)
   * to narrow the sidebar to support-domain nouns on support.auto.dev.
   */
  filter?: (nounName: string) => boolean
}

export interface UseArestResourcesResult {
  resources: ResourceDefinition[]
  isLoading: boolean
  error?: unknown
}

function deriveOpenapiUrl(baseUrl: string, app: string): string {
  const trimmed = baseUrl.replace(/\/$/, '')
  const workerRoot = trimmed.endsWith('/arest')
    ? trimmed.slice(0, -'/arest'.length)
    : trimmed
  return `${workerRoot}/api/openapi.json?app=${encodeURIComponent(app)}`
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}

export function extractNounsFromDoc(doc: unknown): string[] {
  if (!isRecord(doc)) return []
  const components = isRecord(doc.components) ? doc.components : null
  if (!components) return []
  const schemas = isRecord(components.schemas) ? components.schemas : null
  if (!schemas) return []
  const nouns: string[] = []
  for (const [name, schema] of Object.entries(schemas)) {
    if (!isRecord(schema)) continue
    // Only entity schemas — value types and enums are not rendered as
    // independent resources (they're fields on something else).
    if (schema.type !== 'object') continue
    if (!isRecord(schema.properties)) continue
    nouns.push(name)
  }
  return nouns
}

export function useArestResources(
  options: UseArestResourcesOptions,
): UseArestResourcesResult {
  const app = options.app ?? 'ui.do'
  const fetchImpl = options.fetch ?? ((...args) => globalThis.fetch(...args))
  const url = deriveOpenapiUrl(options.baseUrl, app)

  const query = useQuery({
    queryKey: ['arest', 'openapi', app],
    queryFn: async () => {
      const res = await fetchImpl(url, {
        credentials: 'include',
        headers: { accept: 'application/json' },
      })
      if (!res.ok) throw new Error(`openapi fetch failed (HTTP ${res.status})`)
      return res.json()
    },
    staleTime: 5 * 60 * 1000,
  })

  const resources = useMemo<ResourceDefinition[]>(() => {
    if (!query.data) return []
    const nouns = extractNounsFromDoc(query.data)
    const filtered = options.filter ? nouns.filter(options.filter) : nouns
    return filtered.map((noun) => {
      const opts: CreateArestResourceOptions = {
        baseUrl: options.baseUrl,
        app,
        label: options.labels?.[noun],
      }
      return createArestResource(noun, opts)
    })
  }, [query.data, options.baseUrl, app, options.labels, options.filter])

  return {
    resources,
    isLoading: query.isLoading,
    error: query.error ?? undefined,
  }
}
