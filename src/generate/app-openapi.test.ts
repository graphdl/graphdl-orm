import { describe, it, expect } from 'vitest'
import { AppModel } from '../model/app-model'
import { generateOpenAPI } from './openapi'
import type { NounDef, FactTypeDef, ConstraintDef, SpanDef } from '../model/types'

function noun(name: string, domainId: string, objectType: 'entity' | 'value' = 'entity', extra?: Partial<NounDef>): NounDef {
  return { id: name, name, domainId, objectType, ...extra }
}

function ft(id: string, reading: string, nounDefs: Map<string, NounDef>, roles: Array<{ nounName: string; roleIndex: number }>): FactTypeDef {
  return {
    id,
    name: id,
    reading,
    arity: roles.length,
    roles: roles.map(r => ({
      id: `${id}-r${r.roleIndex}`,
      nounName: r.nounName,
      nounDef: nounDefs.get(r.nounName) || { id: r.nounName, name: r.nounName, domainId: '', objectType: 'value' as const },
      roleIndex: r.roleIndex,
    })),
  }
}

function stubDomain(domainId: string, data: {
  nouns?: Map<string, NounDef>
  factTypes?: Map<string, FactTypeDef>
  constraints?: ConstraintDef[]
  constraintSpans?: Map<string, SpanDef[]>
}) {
  return {
    domainId,
    nouns: async () => data.nouns || new Map(),
    factTypes: async () => data.factTypes || new Map(),
    constraints: async () => data.constraints || [],
    constraintSpans: async () => data.constraintSpans || new Map(),
  } as any
}

describe('App → combined OpenAPI spec', () => {
  it('generates schemas from two domains merged into one spec', async () => {
    const vehicleNouns = new Map<string, NounDef>([
      ['Vehicle', noun('Vehicle', 'vehicle-data')],
      ['Make', noun('Make', 'vehicle-data', 'value')],
      ['Model', noun('Model', 'vehicle-data', 'value')],
    ])
    const vehicleFts = new Map<string, FactTypeDef>([
      ['v_make', ft('v_make', 'Vehicle has Make', vehicleNouns, [{ nounName: 'Vehicle', roleIndex: 0 }, { nounName: 'Make', roleIndex: 1 }])],
      ['v_model', ft('v_model', 'Vehicle has Model', vehicleNouns, [{ nounName: 'Vehicle', roleIndex: 0 }, { nounName: 'Model', roleIndex: 1 }])],
    ])
    const vehicleConstraints: ConstraintDef[] = [
      { id: 'uc1', kind: 'UC', modality: 'Alethic', text: 'Each Vehicle has at most one Make', spans: [{ factTypeId: 'v_make', roleIndex: 0 }] },
      { id: 'uc2', kind: 'UC', modality: 'Alethic', text: 'Each Vehicle has at most one Model', spans: [{ factTypeId: 'v_model', roleIndex: 0 }] },
    ]
    const vehicleSpans = new Map<string, SpanDef[]>([
      ['uc1', [{ factTypeId: 'v_make', roleIndex: 0 }]],
      ['uc2', [{ factTypeId: 'v_model', roleIndex: 0 }]],
    ])

    const listingNouns = new Map<string, NounDef>([
      ['Listing', noun('Listing', 'listings')],
      ['Price', noun('Price', 'listings', 'value')],
      ['Vehicle', noun('Vehicle', 'vehicle-data')], // cross-domain reference
    ])
    const listingFts = new Map<string, FactTypeDef>([
      ['l_price', ft('l_price', 'Listing has Price', listingNouns, [{ nounName: 'Listing', roleIndex: 0 }, { nounName: 'Price', roleIndex: 1 }])],
      ['l_vehicle', ft('l_vehicle', 'Listing is for Vehicle', listingNouns, [{ nounName: 'Listing', roleIndex: 0 }, { nounName: 'Vehicle', roleIndex: 1 }])],
    ])
    const listingConstraints: ConstraintDef[] = [
      { id: 'uc3', kind: 'UC', modality: 'Alethic', text: 'Each Listing has at most one Price', spans: [{ factTypeId: 'l_price', roleIndex: 0 }] },
      { id: 'uc4', kind: 'UC', modality: 'Alethic', text: 'Each Listing is for at most one Vehicle', spans: [{ factTypeId: 'l_vehicle', roleIndex: 0 }] },
    ]
    const listingSpans = new Map<string, SpanDef[]>([
      ['uc3', [{ factTypeId: 'l_price', roleIndex: 0 }]],
      ['uc4', [{ factTypeId: 'l_vehicle', roleIndex: 0 }]],
    ])

    const d1 = stubDomain('vehicle-data', { nouns: vehicleNouns, factTypes: vehicleFts, constraints: vehicleConstraints, constraintSpans: vehicleSpans })
    const d2 = stubDomain('listings', { nouns: listingNouns, factTypes: listingFts, constraints: listingConstraints, constraintSpans: listingSpans })

    const app = new AppModel('auto-dev', [d1, d2])
    const openapi = await generateOpenAPI(app)

    // Combined spec should have schemas for both domains
    const schemaNames = Object.keys(openapi.components.schemas)
    expect(schemaNames).toContain('Vehicle')
    expect(schemaNames).toContain('Listing')

    // Vehicle schema has properties from vehicle-data domain
    const vehicleSchema = openapi.components.schemas.Vehicle
    expect(vehicleSchema.properties).toHaveProperty('make')
    expect(vehicleSchema.properties).toHaveProperty('model')

    // Listing schema has properties from listings domain
    const listingSchema = openapi.components.schemas.Listing
    expect(listingSchema.properties).toHaveProperty('price')

    // Cross-domain reference: Listing.vehicle references Vehicle
    expect(listingSchema.properties).toHaveProperty('vehicle')
  })

  it('deduplicates shared nouns across domains', async () => {
    const d1 = stubDomain('d1', { nouns: new Map([['Status', noun('Status', 'd1', 'value', { enumValues: ['active', 'inactive'] })]]) })
    const d2 = stubDomain('d2', { nouns: new Map([['Status', noun('Status', 'd2', 'value', { enumValues: ['active', 'inactive', 'archived'] })]]) })

    const app = new AppModel('app', [d1, d2])
    const nouns = await app.nouns()

    // Last domain wins
    expect(nouns.size).toBe(1)
    expect(nouns.get('Status')?.enumValues).toEqual(['active', 'inactive', 'archived'])
  })
})
