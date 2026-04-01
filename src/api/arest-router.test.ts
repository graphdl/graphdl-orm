import { describe, it, expect } from 'vitest'
import { resolvePath, buildConstraintGraph, nounToSlug, handleRoot, handleArestRequest } from './arest-router'

describe('nounToSlug', () => {
  it('converts noun name to URL slug', () => {
    expect(nounToSlug('Organization')).toBe('organizations')
    expect(nounToSlug('Support Request')).toBe('support-requests')
    expect(nounToSlug('App')).toBe('apps')
  })
})

const mockIR = {
  nouns: {
    Organization: { objectType: 'entity' },
    App: { objectType: 'entity' },
    Domain: { objectType: 'entity' },
    User: { objectType: 'entity' },
    'Support Request': { objectType: 'entity' },
    Name: { objectType: 'value' },
  },
  factTypes: {
    App_belongs_to_Organization: {
      schemaId: 'App_belongs_to_Organization',
      reading: 'App belongs to Organization',
      roles: [{ nounName: 'App', roleIndex: 0 }, { nounName: 'Organization', roleIndex: 1 }],
    },
    Domain_belongs_to_Organization: {
      schemaId: 'Domain_belongs_to_Organization',
      reading: 'Domain belongs to Organization',
      roles: [{ nounName: 'Domain', roleIndex: 0 }, { nounName: 'Organization', roleIndex: 1 }],
    },
    'Support_Request_belongs_to_Domain': {
      schemaId: 'Support_Request_belongs_to_Domain',
      reading: 'Support Request belongs to Domain',
      roles: [{ nounName: 'Support Request', roleIndex: 0 }, { nounName: 'Domain', roleIndex: 1 }],
    },
    Organization_has_Name: {
      schemaId: 'Organization_has_Name',
      reading: 'Organization has Name',
      roles: [{ nounName: 'Organization', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
    },
  },
  constraints: [
    { id: 'uc1', kind: 'UC', spans: [{ factTypeId: 'App_belongs_to_Organization', roleIndex: 0 }] },
    { id: 'uc2', kind: 'UC', spans: [{ factTypeId: 'Domain_belongs_to_Organization', roleIndex: 0 }] },
    { id: 'uc3', kind: 'UC', spans: [{ factTypeId: 'Support_Request_belongs_to_Domain', roleIndex: 0 }] },
    { id: 'uc4', kind: 'UC', spans: [{ factTypeId: 'Organization_has_Name', roleIndex: 0 }] },
  ],
}

describe('buildConstraintGraph', () => {
  it('derives parent-child from UC constraints between entity nouns', () => {
    const graph = buildConstraintGraph(mockIR)
    const orgChildren = graph.children.get('Organization')
    expect(orgChildren).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ noun: 'App', factType: 'App_belongs_to_Organization' }),
        expect.objectContaining({ noun: 'Domain', factType: 'Domain_belongs_to_Organization' }),
      ])
    )
  })

  it('excludes value type nouns from graph', () => {
    const graph = buildConstraintGraph(mockIR)
    const orgChildren = graph.children.get('Organization') || []
    expect(orgChildren.every(c => c.noun !== 'Name')).toBe(true)
  })

  it('handles multi-level nesting', () => {
    const graph = buildConstraintGraph(mockIR)
    const domainChildren = graph.children.get('Domain')
    expect(domainChildren).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ noun: 'Support Request', slug: 'support-requests' }),
      ])
    )
  })
})

describe('resolvePath', () => {
  const graph = buildConstraintGraph(mockIR)

  it('resolves root path', () => {
    const result = resolvePath('/arest/', graph)
    expect(result).toEqual({ level: 'root', segments: [] })
  })

  it('resolves organization collection', () => {
    const result = resolvePath('/arest/organizations', graph)
    expect(result).toEqual({
      level: 'collection',
      noun: 'Organization',
      segments: [{ noun: 'Organization', slug: 'organizations' }],
    })
  })

  it('resolves organization entity', () => {
    const result = resolvePath('/arest/organizations/acme', graph)
    expect(result).toEqual({
      level: 'entity',
      noun: 'Organization',
      id: 'acme',
      segments: [{ noun: 'Organization', slug: 'organizations', id: 'acme' }],
    })
  })

  it('resolves nested child collection', () => {
    const result = resolvePath('/arest/organizations/acme/apps', graph)
    expect(result).toEqual({
      level: 'collection',
      noun: 'App',
      parentNoun: 'Organization',
      parentId: 'acme',
      segments: [
        { noun: 'Organization', slug: 'organizations', id: 'acme' },
        { noun: 'App', slug: 'apps' },
      ],
    })
  })

  it('resolves deeply nested entity', () => {
    const result = resolvePath('/arest/organizations/acme/domains/support/support-requests/sr-123', graph)
    expect(result).toEqual({
      level: 'entity',
      noun: 'Support Request',
      id: 'sr-123',
      parentNoun: 'Domain',
      parentId: 'support',
      segments: [
        { noun: 'Organization', slug: 'organizations', id: 'acme' },
        { noun: 'Domain', slug: 'domains', id: 'support' },
        { noun: 'Support Request', slug: 'support-requests', id: 'sr-123' },
      ],
    })
  })

  it('returns null for invalid child relationship', () => {
    const result = resolvePath('/arest/organizations/acme/support-requests', graph)
    expect(result).toBeNull()
  })
})

describe('handleRoot', () => {
  it('returns user resource with org membership links', async () => {
    const mockGetStub = (id: string) => ({
      get: async () => {
        if (id === 'acme') return { id: 'acme', type: 'Organization', data: { name: 'Acme Corp', slug: 'acme' } }
        if (id === 'other-inc') return { id: 'other-inc', type: 'Organization', data: { name: 'Other Inc', slug: 'other-inc' } }
        return null
      },
    })
    const mockPopulation = {
      facts: {
        User_owns_Organization: [
          { factTypeId: 'User_owns_Organization', bindings: [['User', 'nathan@auto.dev'], ['Organization', 'acme']] },
        ],
        User_belongs_to_Organization: [
          { factTypeId: 'User_belongs_to_Organization', bindings: [['User', 'nathan@auto.dev'], ['Organization', 'other-inc']] },
        ],
      },
    }

    const result = await handleRoot('nathan@auto.dev', mockPopulation, mockGetStub as any)
    expect(result.type).toBe('User')
    expect(result.id).toBe('nathan@auto.dev')
    expect(result._links.self).toEqual({ href: '/arest/' })
    expect(result._links.organizations).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ href: '/arest/organizations/acme', factType: 'User_owns_Organization' }),
        expect.objectContaining({ href: '/arest/organizations/other-inc', factType: 'User_belongs_to_Organization' }),
      ])
    )
  })
})

describe('handleArestRequest', () => {
  const testIR = {
    nouns: {
      Organization: { objectType: 'entity' },
      App: { objectType: 'entity' },
      Name: { objectType: 'value' },
    },
    factTypes: {
      App_belongs_to_Organization: {
        schemaId: 'App_belongs_to_Organization',
        reading: 'App belongs to Organization',
        roles: [{ nounName: 'App', roleIndex: 0 }, { nounName: 'Organization', roleIndex: 1 }],
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
      { id: 'uc1', kind: 'UC', spans: [{ factTypeId: 'App_belongs_to_Organization', roleIndex: 0 }] },
    ],
  }

  const entities: Record<string, any> = {
    acme: { id: 'acme', type: 'Organization', data: { name: 'Acme Corp', slug: 'acme' } },
    'support-app': { id: 'support-app', type: 'App', data: { name: 'Support', slug: 'support-app' } },
  }

  const mockRegistry = {
    getEntityIds: async (type: string) => {
      if (type === 'App') return ['support-app']
      if (type === 'Organization') return ['acme']
      return []
    },
    getEntityCounts: async () => [{ nounType: 'Organization', count: 1 }],
  }

  const mockGetStub = (id: string) => ({
    get: async () => entities[id] || null,
    put: async (input: any) => input,
    delete: async () => ({ id }),
  })

  it('returns entity with derived _links at entity level', async () => {
    const result = await handleArestRequest({
      path: '/arest/organizations/acme',
      method: 'GET',
      ir: testIR,
      registry: mockRegistry as any,
      getStub: mockGetStub as any,
    })
    expect(result).not.toBeNull()
    expect(result.type).toBe('Organization')
    expect(result.id).toBe('acme')
    expect(result._links.self.href).toBe('/arest/organizations/acme')
    expect(result._links.apps).toBeDefined()
    expect(result._links.apps.factType).toBe('App_belongs_to_Organization')
  })

  it('returns collection with _schema at collection level', async () => {
    const result = await handleArestRequest({
      path: '/arest/organizations/acme/apps',
      method: 'GET',
      ir: testIR,
      registry: mockRegistry as any,
      getStub: mockGetStub as any,
    })
    expect(result).not.toBeNull()
    expect(result.docs).toBeDefined()
    expect(result._schema).toBeDefined()
    expect(result._links.self.href).toBe('/arest/organizations/acme/apps')
  })

  it('returns root resource for /arest/', async () => {
    const result = await handleArestRequest({
      path: '/arest/',
      method: 'GET',
      ir: testIR,
      registry: mockRegistry as any,
      getStub: mockGetStub as any,
      userEmail: 'test@example.com',
      population: {
        facts: {
          User_owns_Organization: [
            { factTypeId: 'User_owns_Organization', bindings: [['User', 'test@example.com'], ['Organization', 'acme']] },
          ],
        },
      },
    })
    expect(result).not.toBeNull()
    expect(result.type).toBe('User')
    expect(result._links.self.href).toBe('/arest/')
  })

  it('returns null for invalid path', async () => {
    const result = await handleArestRequest({
      path: '/arest/organizations/acme/nonexistent',
      method: 'GET',
      ir: testIR,
      registry: mockRegistry as any,
      getStub: mockGetStub as any,
    })
    expect(result).toBeNull()
  })
})
