/**
 * openApiSchema unit tests — pure functions, no hooks, no network.
 */
import { describe, expect, it } from 'vitest'
import {
  getFieldsFromSchema,
  getNounSchema,
  humanize,
} from '../openApiSchema'

const sampleDoc = {
  openapi: '3.1.0',
  components: {
    schemas: {
      Organization: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          name: { type: 'string', title: 'Legal Name' },
          taxId: { type: 'string', pattern: '^[0-9-]+$' },
          foundedAt: { type: 'string', format: 'date' },
          active: { type: 'boolean' },
          supportEmail: { type: 'string', format: 'email' },
          website: { type: 'string', format: 'uri' },
          tier: { type: 'string', enum: ['Starter', 'Pro', 'Enterprise'] },
          parent: { $ref: '#/components/schemas/Organization' },
          employeeCount: { type: 'integer' },
        },
        required: ['name', 'taxId'],
      },
    },
  },
}

describe('humanize', () => {
  it.each([
    ['customerEmail', 'Customer Email'],
    ['customer_email', 'Customer Email'],
    ['customer-email', 'Customer Email'],
    ['TaxId', 'Tax Id'],
    ['email', 'Email'],
  ])('humanize(%s) -> %s', (input, expected) => {
    expect(humanize(input)).toBe(expected)
  })
})

describe('getNounSchema', () => {
  it('returns the schema object for a noun by exact name', () => {
    const s = getNounSchema(sampleDoc, 'Organization')
    expect(s).toBeTruthy()
  })

  it('falls back to PascalCase when given a multi-word noun', () => {
    const doc = { components: { schemas: { SupportRequest: { type: 'object' } } } }
    expect(getNounSchema(doc, 'Support Request')).toEqual({ type: 'object' })
  })

  it('returns null for an unknown noun', () => {
    expect(getNounSchema(sampleDoc, 'Martian')).toBeNull()
  })

  it('returns null when the doc is malformed', () => {
    expect(getNounSchema(null, 'Organization')).toBeNull()
    expect(getNounSchema({}, 'Organization')).toBeNull()
    expect(getNounSchema({ components: {} }, 'Organization')).toBeNull()
  })
})

describe('getFieldsFromSchema', () => {
  it('classifies each property by type/format/ref and preserves order', () => {
    const schema = getNounSchema(sampleDoc, 'Organization')
    const fields = getFieldsFromSchema(schema)

    // `id` is intentionally stripped — identity comes from RMAP.
    expect(fields.map((f) => f.name)).toEqual([
      'name', 'taxId', 'foundedAt', 'active', 'supportEmail', 'website', 'tier', 'parent', 'employeeCount',
    ])

    const byName = Object.fromEntries(fields.map((f) => [f.name, f]))
    expect(byName.name.kind).toBe('string')
    expect(byName.name.label).toBe('Legal Name') // title wins over humanized default
    expect(byName.name.required).toBe(true)
    expect(byName.taxId.required).toBe(true)
    expect(byName.foundedAt.kind).toBe('date')
    expect(byName.active.kind).toBe('boolean')
    expect(byName.supportEmail.kind).toBe('email')
    expect(byName.website.kind).toBe('url')
    expect(byName.tier.kind).toBe('enum')
    expect(byName.tier.enum).toEqual(['Starter', 'Pro', 'Enterprise'])
    expect(byName.parent.kind).toBe('reference')
    expect(byName.parent.ref).toBe('Organization')
    expect(byName.employeeCount.kind).toBe('integer')
  })

  it('labels fields without a schema title via humanize(name)', () => {
    const fields = getFieldsFromSchema({
      properties: { customerEmail: { type: 'string', format: 'email' } },
    })
    expect(fields[0].label).toBe('Customer Email')
  })

  it('returns an empty array when the schema has no properties', () => {
    expect(getFieldsFromSchema(null)).toEqual([])
    expect(getFieldsFromSchema({})).toEqual([])
    expect(getFieldsFromSchema({ type: 'object' })).toEqual([])
  })

  describe('iFactr-parallel kind classification', () => {
    it.each([
      ['password',  { type: 'string', format: 'password' }],
      ['time',      { type: 'string', format: 'time' }],
      ['textarea',  { type: 'string', format: 'textarea' }],
      ['textarea',  { type: 'string', format: 'multi-line' }],
    ])('format -> kind=%s', (expected, prop) => {
      const fields = getFieldsFromSchema({ properties: { f: prop } })
      expect(fields[0].kind).toBe(expected)
    })

    it('long strings (maxLength > 255) default to kind=textarea', () => {
      const fields = getFieldsFromSchema({
        properties: { bio: { type: 'string', maxLength: 500 } },
      })
      expect(fields[0].kind).toBe('textarea')
      expect(fields[0].maxLength).toBe(500)
    })

    it('x-widget extension overrides the format-based heuristic', () => {
      // A numeric field with x-widget: slider becomes a range input,
      // matching iFactr.UI's model where the view author substitutes
      // Control types on the same value.
      const fields = getFieldsFromSchema({
        properties: {
          volume: { type: 'integer', minimum: 0, maximum: 100, 'x-widget': 'slider' },
        },
      })
      expect(fields[0].kind).toBe('slider')
      expect(fields[0].min).toBe(0)
      expect(fields[0].max).toBe(100)
    })

    it('x-widget ignores unknown widget names (falls back to default)', () => {
      const fields = getFieldsFromSchema({
        properties: {
          name: { type: 'string', 'x-widget': 'martian-device' },
        },
      })
      expect(fields[0].kind).toBe('string')
    })

    it('x-widget can upgrade a boolean to a switch', () => {
      const fields = getFieldsFromSchema({
        properties: { enabled: { type: 'boolean', 'x-widget': 'switch' } },
      })
      expect(fields[0].kind).toBe('switch')
    })
  })
})
