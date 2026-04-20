/**
 * OpenAPI schema helpers — extract the JSON Schema for a noun from
 * the per-app OpenAPI document (served at /api/openapi.json?app=<name>
 * per #117).
 *
 * The document shape is app-scoped and RMAP-derived: every noun has a
 * `components.schemas.<Noun>` entry whose properties describe the
 * entity's fields. Constraints from the noun's readings (UC, VC, FC,
 * etc.) land as `required`, `enum`, `minLength`, `pattern`, and so on
 * — the same vocabulary JSON Schema already provides. Cross-noun
 * references (fact types) appear as `$ref`.
 *
 * We lift that JSON Schema into a flat `FieldDef[]` so rendering
 * layers (GenericListView, GenericEditView, etc.) can iterate fields
 * without re-parsing the schema each time.
 */

/**
 * Semantic field kinds — mirrors the `Control` subtype list in the
 * readings/ui.md UI domain, which in turn matches the iFactr.UI
 * Control interfaces (IDatePicker, ISelectList, ISlider, etc.). The
 * iFactr.Droid renderer maps each of those to a native Android
 * widget; the web port here maps each one to the richest HTML5
 * control available:
 *
 *   FieldKind      readings/ui.md      iFactr.Droid          Web
 *   ─────────────  ──────────────      ──────────────────    ───────────────────────
 *   string         Text Box            EditText              <input type="text">
 *   textarea       Text Area           EditText (multiline)  <textarea>
 *   password       Password Box        EditText (password)   <input type="password">
 *   email          Text Box            EditText (email)      <input type="email">
 *   url            Text Box            EditText (URL)        <input type="url">
 *   number         (numeric Text Box)  EditText (numeric)    <input type="number">
 *   integer        (numeric Text Box)  EditText (numeric)    <input type="number" step=1>
 *   slider         Slider              SeekBar               <input type="range">
 *   boolean        Checkbox            CheckBox              <input type="checkbox">
 *   switch         Switch              Switch                <input type="checkbox" role="switch">
 *   date           Date Picker         Button+DatePicker     <input type="date">
 *   datetime       Date Picker         Button+DatePicker     <input type="datetime-local">
 *   time           Time Picker         Button+TimePicker     <input type="time">
 *   enum (small)   Select List         Spinner               radio group
 *   enum (large)   Select List         Spinner               <select>
 *   reference      (custom)            (custom)              text with ref title
 *   array/object   (custom)            (custom)              JSON debug fallback
 *
 * An `x-widget` OpenAPI extension overrides the default mapping
 * (e.g. a numeric field with `x-widget: slider` becomes a range
 * input instead of a number input). This matches how iFactr.UI
 * lets a view author substitute a Slider for a Text Box on the
 * same underlying value.
 */
export type FieldKind =
  | 'string'
  | 'textarea'
  | 'password'
  | 'number'
  | 'integer'
  | 'slider'
  | 'boolean'
  | 'switch'
  | 'date'
  | 'datetime'
  | 'time'
  | 'email'
  | 'url'
  | 'enum'
  | 'reference'
  | 'array'
  | 'object'
  | 'unknown'

export interface FieldDef {
  /** Property name (JSON key). */
  name: string
  /** Semantic kind. Drives which input/display component is picked. */
  kind: FieldKind
  /** Whether the field is required (per the schema's `required` list). */
  required: boolean
  /** Enum values if kind === 'enum'. */
  enum?: ReadonlyArray<string | number>
  /** Referenced noun name if kind === 'reference' (via $ref). */
  ref?: string
  /** Short human-readable label — falls back to humanize(name). */
  label: string
  /** Free-text description from the schema (title or description). */
  description?: string
  /** Inclusive lower bound for numeric fields (JSON Schema `minimum`). */
  min?: number
  /** Inclusive upper bound for numeric fields (JSON Schema `maximum`). */
  max?: number
  /** Step size for numeric fields (JSON Schema `multipleOf`). */
  step?: number
  /** String length bounds. */
  minLength?: number
  maxLength?: number
  /** Regex pattern for string fields. */
  pattern?: string
  /** Raw JSON Schema fragment — pass through for callers that need it. */
  raw?: unknown
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}

/** "customerEmail" / "customer_email" / "customer-email" -> "Customer Email". */
export function humanize(propName: string): string {
  const withSpaces = propName
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/[_-]/g, ' ')
    .trim()
  return withSpaces
    .split(/\s+/)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ')
}

/**
 * Drill into an OpenAPI doc and return the JSON Schema for a noun.
 * Looks at `components.schemas[<Noun>]`; tolerates both PascalCase
 * and TitleCase-With-Space variants (e.g. "SupportRequest",
 * "Support Request").
 */
export function getNounSchema(doc: unknown, noun: string): unknown {
  if (!isRecord(doc)) return null
  const components = isRecord(doc.components) ? doc.components : null
  if (!components) return null
  const schemas = isRecord(components.schemas) ? components.schemas : null
  if (!schemas) return null
  if (schemas[noun]) return schemas[noun]
  const pascal = noun.replace(/\s+/g, '')
  if (schemas[pascal]) return schemas[pascal]
  // Final fallback: case-insensitive match.
  for (const [key, value] of Object.entries(schemas)) {
    if (key.toLowerCase() === noun.toLowerCase()) return value
    if (key.toLowerCase() === pascal.toLowerCase()) return value
  }
  return null
}

function classifyProperty(prop: unknown): { kind: FieldKind; enumValues?: ReadonlyArray<string | number>; ref?: string } {
  if (!isRecord(prop)) return { kind: 'unknown' }

  // Explicit `x-widget` wins over format-based heuristics. Matches
  // iFactr.UI's model where the view author picks the concrete
  // Control (e.g. substitute Slider for Text Box on the same value).
  const xWidget = (prop['x-widget'] as string | undefined)
  const widgetOverride = xWidget && isValidKind(xWidget) ? (xWidget as FieldKind) : undefined

  // $ref -> reference field
  if (typeof prop.$ref === 'string') {
    // #/components/schemas/Organization -> Organization
    const tail = (prop.$ref as string).split('/').pop()
    return { kind: widgetOverride ?? 'reference', ref: tail }
  }

  if (Array.isArray(prop.enum)) {
    return { kind: widgetOverride ?? 'enum', enumValues: prop.enum as ReadonlyArray<string | number> }
  }

  if (widgetOverride) return { kind: widgetOverride }

  const type = prop.type
  const format = prop.format

  if (type === 'string') {
    if (format === 'email') return { kind: 'email' }
    if (format === 'uri' || format === 'url') return { kind: 'url' }
    if (format === 'date') return { kind: 'date' }
    if (format === 'date-time' || format === 'datetime') return { kind: 'datetime' }
    if (format === 'time') return { kind: 'time' }
    if (format === 'password') return { kind: 'password' }
    if (format === 'textarea' || format === 'multi-line') return { kind: 'textarea' }
    // Heuristic: very long strings default to a textarea so the admin
    // surface picks a usable control without manual annotation.
    if (typeof prop.maxLength === 'number' && prop.maxLength > 255) return { kind: 'textarea' }
    return { kind: 'string' }
  }
  if (type === 'integer') return { kind: 'integer' }
  if (type === 'number') return { kind: 'number' }
  if (type === 'boolean') return { kind: 'boolean' }
  if (type === 'array') return { kind: 'array' }
  if (type === 'object') return { kind: 'object' }

  return { kind: 'unknown' }
}

/** FieldKind values that x-widget can legally override to. */
const ALL_KINDS: ReadonlySet<string> = new Set([
  'string', 'textarea', 'password', 'number', 'integer', 'slider',
  'boolean', 'switch', 'date', 'datetime', 'time',
  'email', 'url', 'enum', 'reference', 'array', 'object', 'unknown',
])
function isValidKind(s: string): s is FieldKind {
  return ALL_KINDS.has(s)
}

/**
 * Flatten a noun's JSON Schema into an ordered FieldDef list.
 *
 * Property order follows the order of keys in `properties` (OpenAPI
 * documents are JSON-Schema objects so key order is meaningful for
 * display). `id` is excluded from the list because AREST IDs are
 * RMAP-derived — they're the identity of the row, not a field.
 */
export function getFieldsFromSchema(schema: unknown): FieldDef[] {
  if (!isRecord(schema)) return []
  const props = isRecord(schema.properties) ? (schema.properties as Record<string, unknown>) : null
  if (!props) return []
  const requiredList: string[] = Array.isArray(schema.required)
    ? (schema.required as string[]).filter((s): s is string => typeof s === 'string')
    : []

  const fields: FieldDef[] = []
  for (const [name, prop] of Object.entries(props)) {
    if (name === 'id') continue
    const { kind, enumValues, ref } = classifyProperty(prop)
    const description = isRecord(prop)
      ? (prop.description as string | undefined) ?? (prop.title as string | undefined)
      : undefined
    const label = isRecord(prop) && typeof prop.title === 'string'
      ? (prop.title as string)
      : humanize(name)

    const field: FieldDef = {
      name,
      kind,
      required: requiredList.includes(name),
      label,
      raw: prop,
    }
    if (description) field.description = description
    if (enumValues) field.enum = enumValues
    if (ref) field.ref = ref
    if (isRecord(prop)) {
      // Numeric constraints — lift off JSON Schema onto FieldDef so
      // widget layers don't have to re-parse `raw`.
      if (typeof prop.minimum === 'number') field.min = prop.minimum
      if (typeof prop.maximum === 'number') field.max = prop.maximum
      if (typeof prop.multipleOf === 'number') field.step = prop.multipleOf
      if (typeof prop.minLength === 'number') field.minLength = prop.minLength
      if (typeof prop.maxLength === 'number') field.maxLength = prop.maxLength
      if (typeof prop.pattern === 'string') field.pattern = prop.pattern
    }
    fields.push(field)
  }
  return fields
}
