/**
 * useOpenApiSchema(noun) — React hook returning the flattened
 * FieldDef list for a noun, driven by the app-scoped OpenAPI
 * document served at /api/openapi.json?app=<name> (per #117).
 *
 * The underlying OpenAPI document is fetched once per app and cached
 * via TanStack Query; individual nouns extract their fields from the
 * shared document without re-fetching.
 */
import { useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import {
  getFieldsFromSchema,
  getNounSchema,
  type FieldDef,
} from './openApiSchema'

export interface UseOpenApiSchemaOptions {
  /** AREST worker base URL (e.g. https://ui.auto.dev/arest). */
  baseUrl: string
  /** App scope — maps to ?app=<name>. Defaults to 'ui.do'. */
  app?: string
  /** Optional fetch override. Defaults to globalThis.fetch. */
  fetch?: typeof globalThis.fetch
}

export interface UseOpenApiSchemaResult {
  fields: FieldDef[]
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

export function useOpenApiSchema(
  noun: string,
  options: UseOpenApiSchemaOptions,
): UseOpenApiSchemaResult {
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
    // OpenAPI docs change only when the domain recompiles. The SSE
    // bridge could invalidate this in the future; for now a generous
    // stale window is fine — mutations that change schema are rare.
    staleTime: 5 * 60 * 1000,
  })

  const fields = useMemo<FieldDef[]>(() => {
    if (!query.data) return []
    const schema = getNounSchema(query.data, noun)
    return getFieldsFromSchema(schema)
  }, [query.data, noun])

  return {
    fields,
    isLoading: query.isLoading,
    error: query.error ?? undefined,
  }
}
