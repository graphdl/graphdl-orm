/**
 * Per-kind display components used by GenericShowView /
 * GenericListView. Pure formatting; no data fetching.
 *
 * Kind → display mapping:
 *   number / integer  locale-formatted via Intl.NumberFormat
 *   date / datetime   locale-formatted via Intl.DateTimeFormat
 *   boolean           ✓ (Yes) / ✗ (No)
 *   email             mailto: link
 *   url               external link with target=_blank
 *   enum / string     raw text (enum value is already a token)
 *   reference         raw id with a hint that this is a reference
 *   object / array    JSON.stringify — debug-friendly fallback
 */
import type { ReactElement } from 'react'
import type { FieldDef } from '../schema'

export interface SchemaDisplayProps {
  field: FieldDef
  value: unknown
  /** Locale for Intl formatters. Defaults to the browser default. */
  locale?: string
}

const DEFAULT_NUMBER_FMT = new Intl.NumberFormat(undefined)

function formatNumber(value: number, locale?: string): string {
  return locale ? new Intl.NumberFormat(locale).format(value) : DEFAULT_NUMBER_FMT.format(value)
}

function formatDate(value: string, kind: 'date' | 'datetime', locale?: string): string {
  const d = new Date(value)
  if (Number.isNaN(d.getTime())) return value
  const opts: Intl.DateTimeFormatOptions =
    kind === 'date'
      ? { year: 'numeric', month: 'short', day: 'numeric' }
      : { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }
  return new Intl.DateTimeFormat(locale, opts).format(d)
}

export function SchemaDisplay({ field, value, locale }: SchemaDisplayProps): ReactElement {
  if (value == null) return <span data-testid={`display-${field.name}`}>—</span>

  switch (field.kind) {
    case 'number':
    case 'integer':
      return (
        <span data-testid={`display-${field.name}`} data-display="number">
          {typeof value === 'number' ? formatNumber(value, locale) : String(value)}
        </span>
      )

    case 'date':
    case 'datetime':
      return (
        <span data-testid={`display-${field.name}`} data-display={field.kind}>
          {typeof value === 'string' ? formatDate(value, field.kind, locale) : String(value)}
        </span>
      )

    case 'boolean':
      return (
        <span data-testid={`display-${field.name}`} data-display="boolean">
          {value ? '✓ Yes' : '✗ No'}
        </span>
      )

    case 'email': {
      const s = String(value)
      return (
        <a href={`mailto:${s}`} data-testid={`display-${field.name}`} data-display="email">{s}</a>
      )
    }

    case 'url': {
      const s = String(value)
      return (
        <a
          href={s}
          target="_blank"
          rel="noopener noreferrer"
          data-testid={`display-${field.name}`}
          data-display="url"
        >{s}</a>
      )
    }

    case 'enum':
      return <span data-testid={`display-${field.name}`} data-display="enum">{String(value)}</span>

    case 'reference':
      return (
        <span data-testid={`display-${field.name}`} data-display="reference" title={field.ref}>
          {String(value)}
        </span>
      )

    case 'array':
    case 'object':
      return (
        <code data-testid={`display-${field.name}`} data-display={field.kind}>
          {JSON.stringify(value)}
        </code>
      )

    default:
      return <span data-testid={`display-${field.name}`}>{String(value)}</span>
  }
}
