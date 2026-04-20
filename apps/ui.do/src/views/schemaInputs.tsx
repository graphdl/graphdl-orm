/**
 * Minimum-viable input components used by GenericEditView /
 * GenericCreateView. Each renders a plain HTML control keyed by the
 * FieldDef's kind. @mdxui/admin ships richer TextInput / SelectInput
 * etc., but those require a form context we don't yet build; the
 * ResourceDefinition generator (#126) upgrades to those once the
 * field→component mapping is codified.
 */
import type { ReactElement } from 'react'
import type { FieldDef } from '../schema'

export interface SchemaInputProps {
  field: FieldDef
  value: unknown
  onChange: (next: unknown) => void
}

function htmlInputType(field: FieldDef): string {
  switch (field.kind) {
    case 'number':
    case 'integer':
      return 'number'
    case 'boolean':
      return 'checkbox'
    case 'email':
      return 'email'
    case 'url':
      return 'url'
    case 'date':
      return 'date'
    case 'datetime':
      return 'datetime-local'
    default:
      return 'text'
  }
}

export function SchemaInput({ field, value, onChange }: SchemaInputProps): ReactElement {
  if (field.kind === 'enum' && field.enum) {
    return (
      <select
        data-testid={`input-${field.name}`}
        value={typeof value === 'string' || typeof value === 'number' ? String(value) : ''}
        onChange={(e) => onChange(e.target.value)}
      >
        <option value="">--</option>
        {field.enum.map((opt) => (
          <option key={String(opt)} value={String(opt)}>{String(opt)}</option>
        ))}
      </select>
    )
  }

  if (field.kind === 'boolean') {
    return (
      <input
        type="checkbox"
        data-testid={`input-${field.name}`}
        checked={value === true}
        onChange={(e) => onChange(e.target.checked)}
      />
    )
  }

  const type = htmlInputType(field)
  return (
    <input
      type={type}
      data-testid={`input-${field.name}`}
      value={value == null ? '' : String(value)}
      required={field.required}
      onChange={(e) => {
        const raw = e.target.value
        if (field.kind === 'number' || field.kind === 'integer') {
          onChange(raw === '' ? null : Number(raw))
        } else {
          onChange(raw)
        }
      }}
    />
  )
}
