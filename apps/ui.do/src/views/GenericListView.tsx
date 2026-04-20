/**
 * GenericListView — schema-driven list view for any AREST noun.
 *
 * Wraps @mdxui/admin's <ListView> container. Columns are derived
 * from the noun's RMAP-projected JSON Schema (via useOpenApiSchema);
 * rows come from GET /arest/{slug}. A SSE-broadcast mutation on the
 * same noun invalidates ['arest', 'list', slug] keys, so this view
 * auto-refreshes without local polling.
 */
import type { ReactElement } from 'react'
import { ListView } from '@mdxui/admin'
import { useArestList } from '../hooks/useArestResource'
import { useOpenApiSchema, type FieldDef } from '../schema'
import { humanize } from '../schema/openApiSchema'
import { SchemaDisplay } from './schemaDisplay'

export interface GenericListViewProps {
  /** Noun name — e.g. "Organization". Slugified internally. */
  noun: string
  /** AREST worker base URL (e.g. https://ui.auto.dev/arest). */
  baseUrl: string
  /** App scope for /api/openapi.json?app=<name>. Defaults to 'ui.do'. */
  app?: string
  /** Optional page title. Defaults to pluralized humanize(noun). */
  title?: string
  /** Optional pagination. Forwarded to GET /arest/{slug} as page/perPage. */
  pagination?: { page: number; perPage: number }
  /** Render extra buttons in the header (Create, Export). */
  actions?: ReactElement
}


export function GenericListView(props: GenericListViewProps): ReactElement {
  const { noun, baseUrl, app, title, pagination, actions } = props
  const list = useArestList<Record<string, unknown>>(noun, pagination ? { pagination } : undefined, { baseUrl })
  const schema = useOpenApiSchema(noun, { baseUrl, app })

  const isLoading = list.isLoading || schema.isLoading
  const resolvedTitle = title ?? humanize(noun) + 's'

  // Derive a short list of display columns. If schema is empty (e.g. the
  // openapi doc hasn't compiled yet), fall back to the keys of the first
  // row so early boot still shows something useful.
  const rows = list.data?.data ?? []
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

  return (
    <ListView
      title={resolvedTitle}
      actions={actions}
      loading={isLoading}
      empty={<p data-testid="empty-state">No {resolvedTitle.toLowerCase()} yet.</p>}
    >
      {rows.length > 0 && (
        <table data-testid="generic-list-table" style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr>
              <th style={{ textAlign: 'left', padding: '0.5rem', borderBottom: '1px solid #ddd' }}>ID</th>
              {fields.map((f) => (
                <th key={f.name} style={{ textAlign: 'left', padding: '0.5rem', borderBottom: '1px solid #ddd' }}>
                  {f.label}
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
