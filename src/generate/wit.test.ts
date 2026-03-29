import { describe, it, expect } from 'vitest'
import { generateWIT } from './wit'

function mockModel(overrides: Partial<{
  domain: string
  nouns: [string, any][]
  factTypes: [string, any][]
  constraints: any[]
  stateMachines: [string, any][]
}> = {}) {
  const domain = overrides.domain ?? 'test'
  const nouns = new Map(overrides.nouns ?? [])
  const factTypes = new Map(overrides.factTypes ?? [])
  const constraints = overrides.constraints ?? []
  const stateMachines = new Map(overrides.stateMachines ?? [])

  return {
    domain,
    nouns: async () => nouns,
    factTypes: async () => factTypes,
    constraints: async () => constraints,
    stateMachines: async () => stateMachines,
  }
}

describe('generateWIT', () => {
  it('generates package declaration from domain name', async () => {
    const model = mockModel({ domain: 'my-domain' })
    const wit = await generateWIT(model)
    expect(wit).toContain('package graphdl:my-domain;')
  })

  it('generates records from entity types', async () => {
    const model = mockModel({
      nouns: [
        ['Customer', { name: 'Customer', objectType: 'entity' }],
        ['Email', { name: 'Email', objectType: 'value' }],
      ],
      factTypes: [
        ['ft1', { reading: 'Customer has Email', roles: [{ nounName: 'Customer', roleIndex: 0 }, { nounName: 'Email', roleIndex: 1 }] }],
      ],
      constraints: [
        { kind: 'UC', spans: [{ factTypeId: 'ft1', roleIndex: 1 }] },
      ],
    })

    const wit = await generateWIT(model)
    expect(wit).toContain('record customer {')
    expect(wit).toContain('id: string,')
    expect(wit).toContain('email: option<string>,')
  })

  it('generates enums from value types with enum values', async () => {
    const model = mockModel({
      nouns: [
        ['Status', { name: 'Status', objectType: 'value', enumValues: ['active', 'canceled', 'past_due'] }],
      ],
    })

    const wit = await generateWIT(model)
    expect(wit).toContain('enum status {')
    expect(wit).toContain('active,')
    expect(wit).toContain('canceled,')
    expect(wit).toContain('past-due,')
  })

  it('generates list fields for multi-valued fact types (no UC)', async () => {
    const model = mockModel({
      nouns: [
        ['Customer', { name: 'Customer', objectType: 'entity' }],
        ['Order', { name: 'Order', objectType: 'entity' }],
      ],
      factTypes: [
        ['ft1', { reading: 'Customer has Order', roles: [{ nounName: 'Customer', roleIndex: 0 }, { nounName: 'Order', roleIndex: 1 }] }],
      ],
      constraints: [], // no UC → list
    })

    const wit = await generateWIT(model)
    expect(wit).toContain('order: list<order>,')
  })

  it('generates mandatory fields without option wrapper', async () => {
    const model = mockModel({
      nouns: [
        ['Customer', { name: 'Customer', objectType: 'entity' }],
        ['Name', { name: 'Name', objectType: 'value' }],
      ],
      factTypes: [
        ['ft1', { reading: 'Customer has Name', roles: [{ nounName: 'Customer', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }] }],
      ],
      constraints: [
        { kind: 'UC', spans: [{ factTypeId: 'ft1', roleIndex: 1 }] },
        { kind: 'MC', spans: [{ factTypeId: 'ft1', roleIndex: 1 }] },
      ],
    })

    const wit = await generateWIT(model)
    expect(wit).toContain('name: string,')
    // The customer record should have name as non-optional
    const customerRecord = wit.split('record customer {')[1]?.split('}')[0] ?? ''
    expect(customerRecord).not.toContain('option<string>')
  })

  it('includes engine world interface', async () => {
    const model = mockModel()
    const wit = await generateWIT(model)
    expect(wit).toContain('world fol-engine {')
    expect(wit).toContain('export load-ir:')
    expect(wit).toContain('export evaluate:')
    expect(wit).toContain('export forward-chain:')
    expect(wit).toContain('export query:')
    expect(wit).toContain('export apply-command:')
    expect(wit).toContain('export get-transitions:')
  })

  it('includes violation and command-result records', async () => {
    const model = mockModel()
    const wit = await generateWIT(model)
    expect(wit).toContain('record violation {')
    expect(wit).toContain('alethic: bool,')
    expect(wit).toContain('record command-result {')
    expect(wit).toContain('rejected: bool,')
  })
})
