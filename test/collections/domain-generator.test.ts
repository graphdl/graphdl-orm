import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'

let payload: any
let domainAOutput: any
let domainBOutput: any

describe('Domain-scoped generator', () => {
  beforeAll(async () => {
    payload = await initPayload()
    await payload.db.connection.dropDatabase()

    // Domain A: Foo entity with FooName value
    const fooNoun = await payload.create({
      collection: 'nouns',
      data: { name: 'Foo', objectType: 'entity', plural: 'foos', permissions: ['create', 'read'], domain: 'app-a' },
    })
    const fooName = await payload.create({
      collection: 'nouns',
      data: { name: 'FooName', objectType: 'value', valueType: 'string', domain: 'app-a' },
    })
    await payload.update({ collection: 'nouns', id: fooNoun.id, data: { referenceScheme: [fooName.id] } })
    const gs1 = await payload.create({ collection: 'graph-schemas', data: { name: 'FooHasFooName', domain: 'app-a' } })
    await payload.create({ collection: 'readings', data: { text: 'Foo has FooName', graphSchema: gs1.id, domain: 'app-a' } })
    await payload.update({ collection: 'graph-schemas', id: gs1.id, data: { roleRelationship: 'many-to-one' } })

    // Domain B: Bar entity with BarCode value
    const barNoun = await payload.create({
      collection: 'nouns',
      data: { name: 'Bar', objectType: 'entity', plural: 'bars', permissions: ['create', 'read'], domain: 'app-b' },
    })
    const barCode = await payload.create({
      collection: 'nouns',
      data: { name: 'BarCode', objectType: 'value', valueType: 'string', domain: 'app-b' },
    })
    await payload.update({ collection: 'nouns', id: barNoun.id, data: { referenceScheme: [barCode.id] } })
    const gs2 = await payload.create({ collection: 'graph-schemas', data: { name: 'BarHasBarCode', domain: 'app-b' } })
    await payload.create({ collection: 'readings', data: { text: 'Bar has BarCode', graphSchema: gs2.id, domain: 'app-b' } })
    await payload.update({ collection: 'graph-schemas', id: gs2.id, data: { roleRelationship: 'many-to-one' } })

    // Generate for app-a only
    const genA = await payload.create({
      collection: 'generators',
      data: { title: 'App A API', version: '1.0.0', databaseEngine: 'Payload', domain: 'app-a' },
    })
    domainAOutput = genA.output

    // Generate for app-b only
    const genB = await payload.create({
      collection: 'generators',
      data: { title: 'App B API', version: '1.0.0', databaseEngine: 'Payload', domain: 'app-b' },
    })
    domainBOutput = genB.output
  }, 120_000)

  it('should only include schemas from domain A in domain A output', () => {
    const schemas = domainAOutput?.components?.schemas || {}
    expect(schemas.Foo).toBeDefined()
    expect(schemas.Bar).toBeUndefined()
  })

  it('should only include schemas from domain B in domain B output', () => {
    const schemas = domainBOutput?.components?.schemas || {}
    expect(schemas.Bar).toBeDefined()
    expect(schemas.Foo).toBeUndefined()
  })

  it('should include all schemas when domain is not set', async () => {
    const genAll = await payload.create({
      collection: 'generators',
      data: { title: 'All Domains', version: '1.0.0', databaseEngine: 'Payload' },
    })
    const schemas = genAll.output?.components?.schemas || {}
    expect(schemas.Foo).toBeDefined()
    expect(schemas.Bar).toBeDefined()
  })

  it('should include schemas from multiple domains when domains list is set', async () => {
    const genMulti = await payload.create({
      collection: 'generators',
      data: { title: 'Multi Domain', version: '1.0.0', databaseEngine: 'Payload', domains: ['app-a', 'app-b'] },
    })
    const schemas = genMulti.output?.components?.schemas || {}
    expect(schemas.Foo).toBeDefined()
    expect(schemas.Bar).toBeDefined()
  })

  it('should respect domains list over single domain field', async () => {
    const gen = await payload.create({
      collection: 'generators',
      data: { title: 'Domains Override', version: '1.0.0', databaseEngine: 'Payload', domain: 'app-a', domains: ['app-b'] },
    })
    const schemas = gen.output?.components?.schemas || {}
    expect(schemas.Bar).toBeDefined()
    expect(schemas.Foo).toBeUndefined()
  })
})
