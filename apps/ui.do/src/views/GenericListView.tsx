/**
 * GenericListView — schema-driven list view for any AREST noun.
 *
 * Wraps @mdxui/admin's <ListView> container. Columns are derived
 * from the noun's RMAP-projected JSON Schema (via useOpenApiSchema);
 * rows come from GET /arest/{slug}. SSE broadcasts invalidate the
 * same ['arest', 'list', slug] keys so this view auto-refreshes.
 *
 * State lives entirely inside the component (page number, sort,
 * filter). Callers who want external control can compose their
 * own list using useArestList + SchemaDisplay directly.
 */
import { useState, type ReactElement } from 'react'
import { ListView } from '@mdxui/admin'
import { useArestList } from '../hooks/useArestResource'
import { useOpenApiSchema, type FieldDef } from '../schema'
import { humanize } from '../schema/openApiSchema'
import { SchemaDisplay } from './schemaDisplay'

export interface GenericListViewProps {
  /** Noun name — e.g. "Organization". Slugified internally. */
  noun: string
  /** AREST worker base URL. */
  baseUrl: string
  /** App scope for /api/openapi.json?app=<name>. Defaults to 'ui.do'. */
  app?: string
  /** Optional page title. Defaults to pluralized humanize(noun). */
  title?: string
  /** Page size. Defaults to 20. */
  perPage?: number
  /** Render extra buttons in the header (Create, Export). */
  actions?: ReactElement
}

interface ListState {
  page: number
  sort?: { field: string; order: 'ASC' | 'DESC' }
}

export function GenericListView(props: GenericListViewProps): ReactElement {
  const { noun, baseUrl, app, title, perPage = 20, actions } = props
  const [state, setState] = useState<ListState>({ page: 1 })

  const listParams = {
    pagination: { page: state.page, perPage },
    ...(state.sort ? { sort: state.sort } : {}),
  }

  const list = useArestList<Record<string, unknown>>(noun, listParams, { baseUrl })
  const schema = useOpenApiSchema(noun, { baseUrl, app })

  const isLoading = list.isLoading || schema.isLoading
  const resolvedTitle = title ?? humanize(noun) + 's'

  const rows = list.data?.data ?? []
  const total = list.data?.total ?? rows.length
  const pageCount = Math.max(1, Math.ceil(total / perPage))

  const fields: FieldDef[] = schema.fields.length
    ? schema.fields.slice(0, 6)
    : rows[0]
      ? Object.keys(rows[0] as Record<string, unknown>)
          .filter((k) => k !== 'id')
          .slice(0, 6)
          .map((name) => ({
            name,
            kind: 'string' as const,
            required: false,
            label: humanize(name),
          }))
      : []

  const toggleSort = (fieldName: string) => {
    setState((prev) => {
      if (!prev.sort || prev.sort.field !== fieldName) {
        return { ...prev, sort: { field: fieldName, order: 'ASC' }, page: 1 }
      }
      if (prev.sort.order === 'ASC') {
        return { ...prev, sort: { field: fieldName, order: 'DESC' }, page: 1 }
      }
      // Third click clears sort.
      const { sort: _sort, ...rest } = prev
      return { ...rest, page: 1 }
    })
  }

  const sortMarker = (name: string): string => {
    if (!state.sort || state.sort.field !== name) return ''
    return state.sort.order === 'ASC' ? ' ▲' : ' ▼'
  }

  return (
    <ListView
      title={resolvedTitle}
      actions={actions}
      loading={isLoading}
      empty={<p data-testid="empty-state">No {resolvedTitle.toLowerCase()} yet.</p>}
      pagination={
        pageCount > 1 ? (
          <Pagination
            page={state.page}
            pageCount={pageCount}
            total={total}
            onChange={(next) => setState((prev) => ({ ...prev, page: next }))}
          />
        ) : undefined
      }
    >
      {rows.length > 0 && (
        <table data-testid="generic-list-table" style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr>
              <th style={{ textAlign: 'left', padding: '0.5rem', borderBottom: '1px solid #ddd' }}>ID</th>
              {fields.map((f) => (
                <th
                  key={f.name}
                  data-testid={`sort-header-${f.name}`}
                  onClick={() => toggleSort(f.name)}
                  style={{ textAlign: 'left', padding: '0.5rem', borderBottom: '1px solid #ddd', cursor: 'pointer', userSelect: 'none' }}
                >
                  {f.label}{sortMarker(f.name)}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row: Record<string, unknown>, i: number) => (
              <tr key={(row as { id?: string }).id ?? i}>
                <td style={{ padding: '0.5rem', borderBottom: '1px solid #eee' }}>
                  {(row as { id?: string }).id ?? ''}
                </td>
                {fields.map((f) => (
                  <td key={f.name} style={{ padding: '0.5rem', borderBottom: '1px solid #eee' }}>
                    <SchemaDisplay field={f} value={(row as Record<string, unknown>)[f.name]} />
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </ListView>
  )
}

GenericListView.displayName = 'GenericListView'

/**
 * Minimal Prev / N of M / Next control. Rendered in the ListView
 * container's `pagination` slot so it sits below the table.
 */
export interface PaginationProps {
  page: number
  pageCount: number
  total?: number
  onChange: (page: number) => void
}

export function Pagination({ page, pageCount, total, onChange }: PaginationProps): ReactElement {
  return (
    <nav data-testid="list-pagination" aria-label="List pagination" style={{ display: 'flex', gap: '0.5rem', alignItems: 'center' }}>
      <button
        type="button"
        data-testid="paginate-prev"
        disabled={page <= 1}
        onClick={() => onChange(Math.max(1, page - 1))}
      >
        Previous
      </button>
      <span data-testid="paginate-info">
        Page {page} of {pageCount}{total !== undefined && ` · ${total} total`}
      </span>
      <button
        type="button"
        data-testid="paginate-next"
        disabled={page >= pageCount}
        onClick={() => onChange(Math.min(pageCount, page + 1))}
      >
        Next
      </button>
    </nav>
  )
}
