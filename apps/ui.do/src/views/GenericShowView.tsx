/**
 * GenericShowView — schema-driven detail view for any AREST entity.
 *
 * Wraps @mdxui/admin's <ShowView>. Fields are read from the noun's
 * JSON Schema; values from GET /arest/{slug}/{id}. Dates, booleans,
 * and enums render using their schema kind.
 */
import type { ReactElement } from 'react'
import { ShowView } from '@mdxui/admin'
import { useArestOne } from '../hooks/useArestResource'
import { useOpenApiSchema, type FieldDef } from '../schema'
import { humanize } from '../schema/openApiSchema'

export interface GenericShowViewProps {
  noun: string
  id: string
  baseUrl: string
  app?: string
  title?: string
  actions?: ReactElement
  aside?: ReactElement
}

function formatValue(field: FieldDef, value: unknown): string {
  if (value == null) return '—'
  if (field.kind === 'boolean') return value ? 'Yes' : 'No'
  if (typeof value === 'object') return JSON.stringify(value)
  return String(value)
}

export function GenericShowView(props: GenericShowViewProps): ReactElement {
  const { noun, id, baseUrl, app, title, actions, aside } = props
  const entity = useArestOne<Record<string, unknown>>(noun, id, { baseUrl })
  const schema = useOpenApiSchema(noun, { baseUrl, app })
  const isLoading = entity.isLoading || schema.isLoading
  const record = entity.data?.data

  const resolvedTitle = title ?? `${humanize(noun)}: ${id}`

  return (
    <ShowView
      title={resolvedTitle}
      loading={isLoading}
      record={record as Record<string, unknown> | undefined}
      actions={actions}
      aside={aside}
    >
      {record ? (
        <dl data-testid="generic-show-dl" style={{ display: 'grid', gridTemplateColumns: 'max-content 1fr', rowGap: '0.5rem', columnGap: '1rem' }}>
          {schema.fields.map((f) => (
            <div key={f.name} style={{ display: 'contents' }}>
              <dt style={{ fontWeight: 600 }}>{f.label}</dt>
              <dd data-testid={`field-${f.name}`}>{formatValue(f, (record as Record<string, unknown>)[f.name])}</dd>
            </div>
          ))}
        </dl>
      ) : null}
    </ShowView>
  )
}

GenericShowView.displayName = 'GenericShowView'
