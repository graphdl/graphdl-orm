import { describe, expect, it } from 'vitest'
import { extractViolations, mapViolationsToFields, type Violation } from '../violationMapping'
import type { FieldDef } from '../../schema'

function field(name: string, label?: string): FieldDef {
  return { name, kind: 'string', required: false, label: label ?? name }
}

describe('mapViolationsToFields', () => {
  it('matches a violation to the field whose name appears in the reading', () => {
    const violations: Violation[] = [
      { reading: 'Each Customer has at most one email.', detail: 'duplicate email' },
    ]
    const fields = [field('email'), field('name')]
    const map = mapViolationsToFields(violations, fields)
    expect(map.email).toBeDefined()
    expect(map.name).toBeUndefined()
  })

  it('matches via the humanize(name) form (camelCase -> "customer email")', () => {
    const violations: Violation[] = [
      { reading: 'Each Customer must have a unique customer email.' },
    ]
    const fields = [field('customerEmail')]
    const map = mapViolationsToFields(violations, fields)
    expect(map.customerEmail).toBeDefined()
  })

  it('matches via the explicit label', () => {
    const violations: Violation[] = [
      { reading: 'Each Organization has exactly one Legal Name.' },
    ]
    const fields = [field('name', 'Legal Name')]
    const map = mapViolationsToFields(violations, fields)
    expect(map.name).toBeDefined()
  })

  it('assigns the first matching violation when several match the same field', () => {
    const violations: Violation[] = [
      { reading: 'Each Order has a total.', detail: 'first' },
      { reading: 'Each Order has a total.', detail: 'second' },
    ]
    const fields = [field('total')]
    const map = mapViolationsToFields(violations, fields)
    expect(map.total.detail).toBe('first')
  })

  it('returns an empty map when no violation mentions any field', () => {
    const violations: Violation[] = [{ reading: 'generic error' }]
    const fields = [field('name')]
    expect(mapViolationsToFields(violations, fields)).toEqual({})
  })
})

describe('extractViolations', () => {
  it('pulls violations off a flat response', () => {
    const out = extractViolations({ violations: [{ reading: 'x' }] })
    expect(out).toHaveLength(1)
  })

  it('pulls violations off an envelope-nested body', () => {
    const out = extractViolations({ data: { violations: [{ reading: 'x' }] } })
    expect(out).toHaveLength(1)
  })

  it('returns an empty array for anything else', () => {
    expect(extractViolations(null)).toEqual([])
    expect(extractViolations('string')).toEqual([])
    expect(extractViolations({})).toEqual([])
  })
})
