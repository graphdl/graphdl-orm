import { describe, it, expect } from 'vitest'
import { deriveLinks, deriveSchema } from './hateoas'

const mockIR = {
  nouns: {
    Organization: { objectType: 'entity', enumValues: null, superType: null },
    App: { objectType: 'entity', enumValues: null, superType: null },
    Domain: { objectType: 'entity', enumValues: null, superType: null },
    User: { objectType: 'entity', enumValues: null, superType: null },
    Name: { objectType: 'value', enumValues: null, superType: null },
  },
  factTypes: {
    App_belongs_to_Organization: {
      schemaId: 'App_belongs_to_Organization',
      reading: 'App belongs to Organization',
      roles: [{ nounName: 'App', roleIndex: 0 }, { nounName: 'Organization', roleIndex: 1 }],
      readings: [],
    },
    Domain_belongs_to_Organization: {
      schemaId: 'Domain_belongs_to_Organization',
      reading: 'Domain belongs to Organization',
      roles: [{ nounName: 'Domain', roleIndex: 0 }, { nounName: 'Organization', roleIndex: 1 }],
      readings: [],
    },
    User_owns_Organization: {
      schemaId: 'User_owns_Organization',
      reading: 'User owns Organization',
      roles: [{ nounName: 'User', roleIndex: 0 }, { nounName: 'Organization', roleIndex: 1 }],
      readings: [],
    },
    Organization_has_Name: {
      schemaId: 'Organization_has_Name',
      reading: 'Organization has Name',
      roles: [{ nounName: 'Organization', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
      readings: [],
    },
  },
  constraints: [
    { id: 'uc1', kind: 'UC', modality: 'alethic', text: 'Each App belongs to at most one Organization', spans: [{ factTypeId: 'App_belongs_to_Organization', roleIndex: 0 }], deonticOperator: null, entity: null, minOccurrence: null, maxOccurrence: null },
    { id: 'uc2', kind: 'UC', modality: 'alethic', text: 'Each Domain belongs to at most one Organization', spans: [{ factTypeId: 'Domain_belongs_to_Organization', roleIndex: 0 }], deonticOperator: null, entity: null, minOccurrence: null, maxOccurrence: null },
    { id: 'uc3', kind: 'UC', modality: 'alethic', text: 'Each Organization is owned by at most one User', spans: [{ factTypeId: 'User_owns_Organization', roleIndex: 1 }], deonticOperator: null, entity: null, minOccurrence: null, maxOccurrence: null },
    { id: 'uc4', kind: 'UC', modality: 'alethic', text: 'Each Organization has at most one Name', spans: [{ factTypeId: 'Organization_has_Name', roleIndex: 0 }], deonticOperator: null, entity: null, minOccurrence: null, maxOccurrence: null },
  ],
}

describe('deriveLinks', () => {
  it('derives child collection links from UC constraints', () => {
    const links = deriveLinks({
      noun: 'Organization',
      id: 'acme',
      basePath: '/arest/organizations',
      ir: mockIR,
    })
    expect(links.self).toEqual({ href: '/arest/organizations/acme' })
    expect(links.apps).toEqual({ href: '/arest/organizations/acme/apps', factType: 'App_belongs_to_Organization' })
    expect(links.domains).toEqual({ href: '/arest/organizations/acme/domains', factType: 'Domain_belongs_to_Organization' })
  })

  it('derives parent link from UC on own role', () => {
    const links = deriveLinks({
      noun: 'App',
      id: 'support-app',
      basePath: '/arest/organizations/acme/apps',
      ir: mockIR,
      parentPath: '/arest/organizations/acme',
    })
    expect(links.self).toEqual({ href: '/arest/organizations/acme/apps/support-app' })
    expect(links.organization).toEqual({ href: '/arest/organizations/acme', factType: 'App_belongs_to_Organization' })
  })

  it('excludes value type nouns from navigation links', () => {
    const links = deriveLinks({
      noun: 'Organization',
      id: 'acme',
      basePath: '/arest/organizations',
      ir: mockIR,
    })
    expect(links.name).toBeUndefined()
  })

  it('includes transition links when transitions provided', () => {
    const links = deriveLinks({
      noun: 'Organization',
      id: 'acme',
      basePath: '/arest/organizations',
      ir: mockIR,
      transitions: [
        { event: 'archive', targetStatus: 'archived', transitionId: 't1', targetStatusId: 'ts1' },
      ],
    })
    expect(links.archive).toEqual({ href: '/arest/organizations/acme/transition', method: 'POST' })
  })

  it('derives collection links (no id)', () => {
    const links = deriveLinks({
      noun: 'Organization',
      basePath: '/arest/organizations',
      ir: mockIR,
    })
    expect(links.self).toEqual({ href: '/arest/organizations' })
    expect(links.create).toEqual({ href: '/arest/organizations', method: 'POST' })
  })
})

describe('deriveSchema', () => {
  it('derives fields from fact types involving the noun', () => {
    const schema = deriveSchema('Organization', mockIR)
    expect(schema.fields).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ name: 'Name', role: 'attribute', factType: 'Organization_has_Name' }),
      ])
    )
  })

  it('marks fields required when MC constraint exists', () => {
    const irWithMC = {
      ...mockIR,
      constraints: [
        ...mockIR.constraints,
        { id: 'mc1', kind: 'MC', modality: 'alethic', text: 'Each Organization has at least one Name', spans: [{ factTypeId: 'Organization_has_Name', roleIndex: 0 }], deonticOperator: null, entity: null, minOccurrence: null, maxOccurrence: null },
      ],
    }
    const schema = deriveSchema('Organization', irWithMC)
    const nameField = schema.fields.find((f: any) => f.name === 'Name')
    expect(nameField?.required).toBe(true)
  })

  it('marks entity-type fields as reference role', () => {
    const schema = deriveSchema('App', mockIR)
    const orgField = schema.fields.find((f: any) => f.name === 'Organization')
    expect(orgField?.role).toBe('reference')
    expect(orgField?.factType).toBe('App_belongs_to_Organization')
  })

  it('includes applicable constraints', () => {
    const schema = deriveSchema('App', mockIR)
    expect(schema.constraints).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ text: 'Each App belongs to at most one Organization', kind: 'UC' }),
      ])
    )
  })
})
