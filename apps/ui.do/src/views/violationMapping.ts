/**
 * Map Theorem-5 violations back to the form fields they constrain.
 *
 * The violation carries:
 *   { reading, constraintId, modality: 'alethic'|'deontic', detail? }
 * where `reading` is the original FORML 2 sentence the constraint
 * compiled from (whitepaper Corollary 1). The sentence mentions the
 * nouns and roles the constraint involves; heuristic matching
 * against a field's name / humanize(name) / label is good enough
 * to land the error on the right input.
 *
 * Returns a map from field name to the first matching violation.
 * Fields not mentioned in any violation stay out of the map; those
 * violations still surface as the top-level form error.
 */
import type { FieldDef } from '../schema'

export interface Violation {
  reading?: string
  constraintId?: string
  modality?: 'alethic' | 'deontic'
  detail?: string
}

function wordify(s: string): string {
  return s
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/[_-]/g, ' ')
    .toLowerCase()
}

function matchesField(field: FieldDef, reading: string): boolean {
  const hay = reading.toLowerCase()
  const tokens = [
    field.name.toLowerCase(),
    wordify(field.name),
    field.label.toLowerCase(),
  ]
  for (const token of tokens) {
    if (!token) continue
    // Word-boundary match so "name" doesn't match "noun name".
    const re = new RegExp(`\\b${escapeRegex(token)}\\b`)
    if (re.test(hay)) return true
  }
  return false
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

export function mapViolationsToFields(
  violations: readonly Violation[],
  fields: readonly FieldDef[],
): Record<string, Violation> {
  const out: Record<string, Violation> = {}
  for (const v of violations) {
    const text = v.reading ?? v.detail ?? ''
    if (!text) continue
    for (const f of fields) {
      if (out[f.name]) continue
      if (matchesField(f, text)) out[f.name] = v
    }
  }
  return out
}

/**
 * Extract the violations list out of a thrown error's message or
 * a 422 response body. The data provider throws with the violation's
 * detail as the message, so callers need to walk the original
 * response to recover the full list. This helper accepts both the
 * response body and the Error directly; returns an empty array when
 * it can't find a violations array.
 */
export function extractViolations(source: unknown): Violation[] {
  if (!source) return []
  if (typeof source === 'object' && source !== null) {
    const rec = source as Record<string, unknown>
    if (Array.isArray(rec.violations)) return rec.violations as Violation[]
    if ('data' in rec && rec.data && typeof rec.data === 'object') {
      const data = rec.data as Record<string, unknown>
      if (Array.isArray(data.violations)) return data.violations as Violation[]
    }
  }
  return []
}
