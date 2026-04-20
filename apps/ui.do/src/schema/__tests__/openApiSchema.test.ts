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
})
