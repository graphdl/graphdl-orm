/**
 * Per-kind input components used by GenericEditView / GenericCreateView.
 *
 * Kind → widget mapping (keyed off FieldDef.kind + schema constraints):
 *   number / integer  <input type="number">  with step/min/max from
 *                     JSON Schema's multipleOf / minimum / maximum
 *   boolean           <input type="checkbox">
 *   date              <input type="date">
 *   datetime          <input type="datetime-local">
 *   email             <input type="email">
 *   url               <input type="url">
 *   enum, <=4 opts    radio group
 *   enum, >4 opts     <select>
 *   string            <input type="text"> with minLength/maxLength/
 *                     pattern lifted to HTML5 validation attrs
 *
 * The radio-vs-select threshold matches common admin conventions:
 * with <=4 options radio buttons scan faster than a dropdown; past
 * that the radio list gets visually noisy and we fall to a select.
 * Consumers can override via `enumAsRadioThreshold`.
 */
import type { ReactElement } from 'react'
import type { FieldDef } from '../schema'

export interface SchemaInputProps {
  field: FieldDef
  value: unknown
  onChange: (next: unknown) => void
  /** Below this option count, enums render as radio buttons. Default 4. */
  enumAsRadioThreshold?: number
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

function defaultStep(field: FieldDef): number | undefined {
  if (field.step !== undefined) return field.step
  if (field.kind === 'integer') return 1
  return undefined
}

export function SchemaInput({
  field,
  value,
  onChange,
  enumAsRadioThreshold = 4,
}: SchemaInputProps): ReactElement {
  // ── Enum: radio for small sets, select for big ones ─────────────
  if (field.kind === 'enum' && field.enum) {
    const opts = field.enum
    const current = typeof value === 'string' || typeof value === 'number' ? String(value) : ''

    if (opts.length <= enumAsRadioThreshold) {
      return (
        <fieldset
          data-testid={`input-${field.name}`}
          data-widget="radio-group"
          style={{ border: 'none', padding: 0, margin: 0 }}
        >
          <legend className="sr-only">{field.label}</legend>
          {opts.map((opt) => {
            const v = String(opt)
            const id = `radio-${field.name}-${v}`
            return (
              <label
                key={v}
                htmlFor={id}
                style={{ marginRight: '1rem', display: 'inline-flex', alignItems: 'center', gap: '0.25rem' }}
              >
                <input
                  id={id}
                  type="radio"
                  name={field.name}
                  value={v}
                  checked={current === v}
                  onChange={() => onChange(v)}
                />
                {v}
              </label>
            )
          })}
        </fieldset>
      )
    }

    return (
      <select
        data-testid={`input-${field.name}`}
        data-widget="select"
        value={current}
        onChange={(e) => onChange(e.target.value)}
      >
        <option value="">--</option>
        {opts.map((opt) => (
          <option key={String(opt)} value={String(opt)}>{String(opt)}</option>
        ))}
      </select>
    )
  }

  // ── Boolean / switch (iFactr: CheckBox vs Switch are distinct) ──
  if (field.kind === 'boolean' || field.kind === 'switch') {
    const isSwitch = field.kind === 'switch'
    return (
      <input
        type="checkbox"
        data-testid={`input-${field.name}`}
        data-widget={isSwitch ? 'switch' : 'checkbox'}
        role={isSwitch ? 'switch' : undefined}
        checked={value === true}
        onChange={(e) => onChange(e.target.checked)}
      />
    )
  }

  // ── Slider (iFactr: Slider → Android SeekBar → HTML range) ──────
  if (field.kind === 'slider') {
    const step = defaultStep(field)
    return (
      <input
        type="range"
        data-testid={`input-${field.name}`}
        data-widget="slider"
        value={value == null ? (field.min ?? 0) : Number(value)}
        min={field.min}
        max={field.max}
        step={step}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    )
  }

  // ── Number / integer (iFactr: numeric Text Box → EditText) ──────
  if (field.kind === 'number' || field.kind === 'integer') {
    const step = defaultStep(field)
    return (
      <input
        type="number"
        data-testid={`input-${field.name}`}
        data-widget="number"
        value={value == null ? '' : String(value)}
        required={field.required}
        min={field.min}
        max={field.max}
        step={step}
        onChange={(e) => {
          const raw = e.target.value
          onChange(raw === '' ? null : Number(raw))
        }}
      />
    )
  }

  // ── Text Area (iFactr: Text Area → multi-line EditText) ─────────
  if (field.kind === 'textarea') {
    return (
      <textarea
        data-testid={`input-${field.name}`}
        data-widget="textarea"
        value={value == null ? '' : String(value)}
        required={field.required}
        minLength={field.minLength}
        maxLength={field.maxLength}
        rows={4}
        onChange={(e) => onChange(e.target.value)}
      />
    )
  }

  // ── Password Box (iFactr: Password Box → EditText w/ password) ──
  if (field.kind === 'password') {
    return (
      <input
        type="password"
        data-testid={`input-${field.name}`}
        data-widget="password"
        value={value == null ? '' : String(value)}
        required={field.required}
        minLength={field.minLength}
        maxLength={field.maxLength}
        onChange={(e) => onChange(e.target.value)}
      />
    )
  }

  // ── Time (iFactr: Time Picker → Button+TimePickerDialog) ────────
  if (field.kind === 'time') {
    return (
      <input
        type="time"
        data-testid={`input-${field.name}`}
        data-widget="time"
        value={value == null ? '' : String(value)}
        required={field.required}
        onChange={(e) => onChange(e.target.value)}
      />
    )
  }

  // ── Text Box / email / url / date / datetime ────────────────────
  const type = htmlInputType(field)
  return (
    <input
      type={type}
      data-testid={`input-${field.name}`}
      data-widget={type}
      value={value == null ? '' : String(value)}
      required={field.required}
      minLength={field.minLength}
      maxLength={field.maxLength}
      pattern={field.pattern}
      onChange={(e) => onChange(e.target.value)}
    />
  )
}
