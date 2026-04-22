/**
 * External System resource surface (#343).
 *
 * Sibling to `createArestResource` / `useArestResources` — but for
 * mounted external vocabularies rather than first-class nouns.
 * Discovery reads `/external/{system}/types` paths out of the
 * app-scoped OpenAPI doc (emitted by generators/openapi.rs once the
 * tenant has mounted any External System).
 *
 * Each mounted system produces a ResourceDefinition with:
 *   - list view: fetches GET /external/{system}/types, renders a
 *     grid of type names that link into the show view.
 *   - show view: fetches GET /external/{system}/types/{name}, renders
 *     the BrowseResponse as property table + subtype chain.
 *   - no create / edit (vocabularies are read-only).
 */
import type { ComponentType, ReactElement, ReactNode } from 'react'
import { useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import { useCurrentResource } from '@mdxui/admin'
import type { ResourceDefinition } from './resourceDefinition'

export interface CreateExternalSystemResourceOptions {
  /** AREST worker base URL. */
  baseUrl: string
  /** App scope for /api/openapi.json?app=<name>. Defaults to 'ui.do'. */
  app?: string
  /** Override the sidebar label; defaults to the system name. */
  label?: string
  /** Sidebar icon. */
  icon?: ReactNode
  /** Hide the resource from the menu. */
  hideFromMenu?: boolean
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}

/**
 * Derive the set of mounted External System names by walking path
 * keys `/external/<system>/types` in the OpenAPI doc. Returns names
 * in the order they first appear, deduplicated across collection +
 * item paths.
 */
export function extractExternalSystemsFromDoc(doc: unknown): string[] {
  if (!isRecord(doc)) return []
  const paths = isRecord(doc.paths) ? doc.paths : null
  if (!paths) return []
  const seen = new Set<string>()
  const out: string[] = []
  for (const path of Object.keys(paths)) {
    const match = /^\/external\/([^/]+)\/types(\/\{name\})?$/.exec(path)
    if (!match) continue
    const system = match[1]
    if (seen.has(system)) continue
    seen.add(system)
    out.push(system)
  }
  return out
}

/** Convert a system name to a URL-safe resource slug. */
function systemToSlug(system: string): string {
  const safe = system.replace(/[^a-zA-Z0-9]+/g, '-').replace(/^-+|-+$/g, '').toLowerCase()
  return `external-${safe}`
}

function deriveExternalRoot(baseUrl: string): string {
  const trimmed = baseUrl.replace(/\/$/, '')
  return trimmed.endsWith('/arest')
    ? trimmed.slice(0, -'/arest'.length)
    : trimmed
}

interface BrowseResponse {
  type: string
  supertypes: string[]
  subtypes: string[]
  properties: Array<{ name: string; range: string }>
}

function useExternalTypeList(baseUrl: string, system: string) {
  const root = deriveExternalRoot(baseUrl)
  const url = `${root}/external/${encodeURIComponent(system)}/types`
  return useQuery({
    queryKey: ['arest', 'external', system, 'types'],
    queryFn: async (): Promise<string[]> => {
      const res = await fetch(url, { credentials: 'include', headers: { accept: 'application/json' } })
      if (!res.ok) throw new Error(`external types fetch failed (HTTP ${res.status})`)
      const json = await res.json()
      return Array.isArray(json) ? json.filter((v): v is string => typeof v === 'string') : []
    },
    staleTime: 5 * 60 * 1000,
  })
}

function useExternalBrowse(baseUrl: string, system: string, typeName: string) {
  const root = deriveExternalRoot(baseUrl)
  const url = `${root}/external/${encodeURIComponent(system)}/types/${encodeURIComponent(typeName)}`
  return useQuery({
    queryKey: ['arest', 'external', system, 'browse', typeName],
    queryFn: async (): Promise<BrowseResponse | null> => {
      const res = await fetch(url, { credentials: 'include', headers: { accept: 'application/json' } })
      if (res.status === 404) return null
      if (!res.ok) throw new Error(`external browse fetch failed (HTTP ${res.status})`)
      const json = await res.json() as Partial<BrowseResponse>
      return {
        type: typeof json.type === 'string' ? json.type : typeName,
        supertypes: Array.isArray(json.supertypes) ? json.supertypes.filter((s): s is string => typeof s === 'string') : [],
        subtypes:   Array.isArray(json.subtypes)   ? json.subtypes.filter((s): s is string => typeof s === 'string')   : [],
        properties: Array.isArray(json.properties)
          ? (json.properties as Array<{ name: unknown; range: unknown }>)
              .filter((p) => typeof p.name === 'string' && typeof p.range === 'string')
              .map((p) => ({ name: p.name as string, range: p.range as string }))
          : [],
      }
    },
    enabled: typeName.length > 0,
    staleTime: 5 * 60 * 1000,
  })
}

/** List view: one row per type name exposed by the system. */
function ExternalSystemListView({ system, baseUrl }: { system: string; baseUrl: string }): ReactElement {
  const query = useExternalTypeList(baseUrl, system)
  const slug = systemToSlug(system)
  if (query.isLoading) return <div data-testid="list-container">Loading…</div>
  if (query.error) return <div data-testid="list-container">Failed to load types for {system}.</div>
  const types = query.data ?? []
  return (
    <div data-testid="list-container">
      <h1>{system}</h1>
      <ul data-testid={`external-types-${slug}`}>
        {types.map((t) => (
          <li key={t}>
            <a href={`#/${slug}/${encodeURIComponent(t)}/show`} data-testid={`external-type-${t}`}>
              {t}
            </a>
          </li>
        ))}
      </ul>
    </div>
  )
}

/** Show view: supertypes chain + subtypes + property table. */
function ExternalSystemShowView({ system, typeName, baseUrl }: { system: string; typeName: string; baseUrl: string }): ReactElement {
  const query = useExternalBrowse(baseUrl, system, typeName)
  if (query.isLoading) return <div data-testid="show-container">Loading {typeName}…</div>
  if (query.error) return <div data-testid="show-container">Failed to load {typeName}.</div>
  const data = query.data
  if (!data) return <div data-testid="show-container">Unknown type '{typeName}' in {system}.</div>
  return (
    <div data-testid="show-container">
      <h1>{data.type}</h1>
      {data.supertypes.length > 0 && (
        <p data-testid="supertypes">
          Supertypes: {data.supertypes.join(' ← ')}
        </p>
      )}
      {data.subtypes.length > 0 && (
        <p data-testid="subtypes">
          Subtypes: {data.subtypes.join(', ')}
        </p>
      )}
      <table data-testid="properties">
        <thead><tr><th>Property</th><th>Range</th></tr></thead>
        <tbody>
          {data.properties.map((p) => (
            <tr key={p.name}>
              <td>{p.name}</td>
              <td>{p.range}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

/**
 * Build a ResourceDefinition that mounts list + show views for a
 * single mounted External System.
 */
export function createExternalSystemResource(
  system: string,
  options: CreateExternalSystemResourceOptions,
): ResourceDefinition {
  const slug = systemToSlug(system)

  const ListComponent: ComponentType = () => (
    <ExternalSystemListView system={system} baseUrl={options.baseUrl} />
  )
  ListComponent.displayName = `ExternalSystem(${system})ListView`

  const ShowComponent: ComponentType = () => {
    const { recordId } = useCurrentResource()
    return <ExternalSystemShowView system={system} typeName={recordId ?? ''} baseUrl={options.baseUrl} />
  }
  ShowComponent.displayName = `ExternalSystem(${system})ShowView`

  return {
    name: slug,
    list: ListComponent,
    show: ShowComponent,
    icon: options.icon,
    options: {
      label: options.label ?? system,
      hideFromMenu: options.hideFromMenu ?? false,
    },
  }
}

export interface UseExternalSystemsOptions {
  baseUrl: string
  app?: string
  fetch?: typeof globalThis.fetch
  labels?: Record<string, string>
}

export interface UseExternalSystemsResult {
  resources: ResourceDefinition[]
  systems: string[]
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

/**
 * Discover every mounted External System in the app-scoped OpenAPI
 * doc and build one ResourceDefinition per system. Returns the raw
 * system names too so the AppShell can render a grouped nav section.
 */
export function useExternalSystems(
  options: UseExternalSystemsOptions,
): UseExternalSystemsResult {
  const app = options.app ?? 'ui.do'
  const fetchImpl = options.fetch ?? ((...args) => globalThis.fetch(...args))
  const url = deriveOpenapiUrl(options.baseUrl, app)

  const query = useQuery({
    queryKey: ['arest', 'openapi', app, 'external-systems'],
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

  const systems = useMemo(
    () => (query.data ? extractExternalSystemsFromDoc(query.data) : []),
    [query.data],
  )

  const resources = useMemo<ResourceDefinition[]>(() => {
    return systems.map((system) => createExternalSystemResource(system, {
      baseUrl: options.baseUrl,
      app,
      label: options.labels?.[system],
    }))
  }, [systems, options.baseUrl, app, options.labels])

  return {
    systems,
    resources,
    isLoading: query.isLoading,
    error: query.error ?? undefined,
  }
}
