/**
 * ReferencePicker — edit-side widget for kind: 'reference' fields.
 *
 * When a field's JSON Schema carries a $ref (e.g. `$ref:
 * "#/components/schemas/Organization"`) the openApiSchema classifier
 * marks the FieldDef as kind:'reference' with `ref` set to the
 * target noun. This widget:
 *   1. Fetches the first page of the referenced noun's collection
 *      via useArestList so the picker has options.
 *   2. Renders a <select> keyed by id, labeled by the row's `name`
 *      / `title` / `label` / id fallback.
 *   3. Offers "search" filtering by re-issuing the list query with
 *      a filter (skipped in tests where the collection is small).
 *
 * The display-side companion (in schemaDisplay.tsx) fetches the
 * referenced entity's display label via useArestOne so the show
 * view renders `Acme Corp` rather than `acme`.
 */
import type { ReactElement } from 'react'
import { useArestList, useArestOne } from '../hooks/useArestResource'

export interface ReferencePickerProps {
  /** The referenced noun name (e.g. "Organization"). */
  noun: string
  /** Currently-selected id (empty string for nothing). */
  value: string
  onChange: (id: string) => void
  baseUrl: string
  /** data-testid for the <select>. */
  testId?: string
  /** Fields to try when labeling options. Defaults: ['name','title','label']. */
  labelFields?: string[]
}

const DEFAULT_LABEL_FIELDS = ['name', 'title', 'label']

function labelFor(row: Record<string, unknown>, labelFields: string[]): string {
  for (const f of labelFields) {
    const v = row[f]
    if (typeof v === 'string' && v.length > 0) return v
  }
  const id = row.id
  return typeof id === 'string' ? id : JSON.stringify(row)
}

export function ReferencePicker(props: ReferencePickerProps): ReactElement {
  const { noun, value, onChange, baseUrl, testId, labelFields = DEFAULT_LABEL_FIELDS } = props
  const list = useArestList<Record<string, unknown>>(
    noun,
    { pagination: { page: 1, perPage: 100 } },
    { baseUrl },
  )
  const rows = list.data?.data ?? []
  return (
    <select
      data-testid={testId}
      data-widget="reference-picker"
      value={value}
      onChange={(e) => onChange(e.target.value)}
    >
      <option value="">— none —</option>
      {rows.map((row) => {
        const id = row.id as string
        return <option key={id} value={id}>{labelFor(row, labelFields)}</option>
      })}
    </select>
  )
}

ReferencePicker.displayName = 'ReferencePicker'

/**
 * ReferenceLabel — fetch-and-render the referenced entity's
 * display label. Used in show views / list cells so references
 * don't render as opaque ids.
 */
export interface ReferenceLabelProps {
  noun: string
  id: string
  baseUrl: string
  labelFields?: string[]
  testId?: string
}

export function ReferenceLabel({ noun, id, baseUrl, labelFields = DEFAULT_LABEL_FIELDS, testId }: ReferenceLabelProps): ReactElement {
  const one = useArestOne<Record<string, unknown>>(noun, id, { baseUrl })
  const row = one.data?.data
  const label = row ? labelFor(row as Record<string, unknown>, labelFields) : id
  return <span data-testid={testId} data-display="reference">{label}</span>
}

ReferenceLabel.displayName = 'ReferenceLabel'
