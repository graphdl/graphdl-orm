/**
 * BlocksPage — content pages rendered from AREST ui-domain entities
 * through mdxui-style block components.
 *
 * Per readings/ui.md the UI domain models content as `View` + `Section`
 * + `Cell` entities. A "content page" in the site sense is a sequence
 * of Sections (or whatever noun a consumer chooses) that each map to
 * a specific block component via `type`. The BlocksPage fetches the
 * noun's list, sorts by a display order, looks each row up in a
 * block registry, and renders.
 *
 * `@mdxui/blocks` doesn't ship as an installed package today — mdxui
 * surfaces the same block primitives via its site / app namespaces
 * (Hero, Features, Pricing, etc.). Consumers supply a registry that
 * maps block `type` strings to mdxui components; the default
 * registry ships plain-HTML fallbacks so the page renders in jsdom
 * and can be styled downstream.
 */
import type { ComponentType, ReactElement } from 'react'
import { useArestList } from '../hooks/useArestResource'
import { useArestOne } from '../hooks/useArestResource'
import type { ArestResourceOptions } from '../hooks/useArestResource'

/**
 * A block row is one entity in the UI domain whose `type` picks
 * which component renders it. Extra props flow through as `props`.
 */
export interface BlockRow {
  id: string
  type?: string
  order?: number
  [key: string]: unknown
}

export type BlockRegistry = Record<string, ComponentType<{ row: BlockRow }>>

/**
 * Default fallback registry. Each block type renders as a plain
 * section with the row's fields. Callers replace entries with
 * mdxui primitives (Hero, Features, Pricing…) as needed.
 */
export const DEFAULT_BLOCK_REGISTRY: BlockRegistry = {
  hero: ({ row }) => (
    <section data-testid={`block-hero-${row.id}`} data-block-type="hero">
      <h1>{String(row.title ?? '')}</h1>
      {row.subtitle != null && <p>{String(row.subtitle)}</p>}
    </section>
  ),
  features: ({ row }) => (
    <section data-testid={`block-features-${row.id}`} data-block-type="features">
      <h2>{String(row.title ?? 'Features')}</h2>
      {Array.isArray(row.items) && (
        <ul>
          {(row.items as Array<{ label: string }>).map((item, i) => (
            <li key={i}>{item.label}</li>
          ))}
        </ul>
      )}
    </section>
  ),
  text: ({ row }) => (
    <section data-testid={`block-text-${row.id}`} data-block-type="text">
      {row.title != null && <h2>{String(row.title)}</h2>}
      {row.body != null && <p>{String(row.body)}</p>}
    </section>
  ),
}

export interface BlocksPageProps {
  /** Noun name whose rows form the page's block sequence. */
  noun: string
  /** AREST worker base URL. */
  baseUrl: string
  /**
   * Optional parent entity — when provided the page fetches the
   * noun's rows as a child collection of the parent
   * (/arest/{parent-slug}/{parent-id}/{noun-slug}), otherwise the
   * flat collection.
   */
  parent?: { noun: string; id: string }
  /** Block registry. Defaults to DEFAULT_BLOCK_REGISTRY. */
  registry?: BlockRegistry
  /** Fallback component when a row's type isn't in the registry. */
  fallback?: ComponentType<{ row: BlockRow }>
  /** Field name that carries the block type. Defaults to 'type'. */
  typeField?: string
  /** Field name that sorts rows. Defaults to 'order'. */
  orderField?: string
}

function compareOrder(a: BlockRow, b: BlockRow, field: string): number {
  const av = (a as Record<string, unknown>)[field]
  const bv = (b as Record<string, unknown>)[field]
  const an = typeof av === 'number' ? av : Number.MAX_SAFE_INTEGER
  const bn = typeof bv === 'number' ? bv : Number.MAX_SAFE_INTEGER
  return an - bn
}

const DefaultFallback: ComponentType<{ row: BlockRow }> = ({ row }) => (
  <section data-testid={`block-unknown-${row.id}`} data-block-type="unknown">
    <pre>{JSON.stringify(row, null, 2)}</pre>
  </section>
)

export function BlocksPage(props: BlocksPageProps): ReactElement {
  const {
    noun,
    baseUrl,
    parent,
    registry = DEFAULT_BLOCK_REGISTRY,
    fallback = DefaultFallback,
    typeField = 'type',
    orderField = 'order',
  } = props

  const opts: ArestResourceOptions = { baseUrl }
  // Parent context is optional — when the caller knows the page is
  // a child collection of an entity, useArestOne primes the parent
  // record so loading/isLoading aligns across both fetches.
  const parentQuery = useArestOne(parent?.noun ?? '', parent?.id ?? '', opts)
  const listFilter = parent ? { [`belongs to ${parent.noun}`]: parent.id } : undefined
  const listQuery = useArestList<BlockRow>(noun, listFilter ? { filter: listFilter } : undefined, opts)

  const isLoading = (parent && parentQuery.isLoading) || listQuery.isLoading
  const rows = [...(listQuery.data?.data ?? [])].sort((a, b) => compareOrder(a, b, orderField))

  return (
    <article data-testid="blocks-page">
      {isLoading && <p>Loading…</p>}
      {rows.map((row) => {
        const kind = (row as Record<string, unknown>)[typeField]
        const key = typeof kind === 'string' ? kind.toLowerCase() : ''
        const Component = registry[key] ?? fallback
        return <Component key={row.id} row={row} />
      })}
    </article>
  )
}

BlocksPage.displayName = 'BlocksPage'
