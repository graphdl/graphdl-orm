import { describe, it, expect, beforeAll } from 'vitest'
import { ingestReading, ingestClaims, type ExtractedClaims } from './ingest'

let payload: any

describe('ingestReading', () => {
  let domainId: string

  beforeAll(async () => {
    const { initPayload } = await import('../../test/helpers/initPayload')
    payload = await initPayload()
    await payload.db.connection.dropDatabase()

    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'test-ingest-reading', name: 'Test Ingest Reading' },
    })
    domainId = domain.id

    // Pre-create nouns so tokenizer can find them
    await payload.create({ collection: 'nouns', data: { name: 'Customer', objectType: 'entity', domain: domainId } })
    await payload.create({ collection: 'nouns', data: { name: 'Email', objectType: 'value', domain: domainId } })
  }, 120_000)

  it('creates a graph schema and reading from text', async () => {
    const result = await ingestReading(payload, {
      text: 'Customer has Email',
      domainId,
    })

    expect(result.errors).toHaveLength(0)
    expect(result.graphSchemaId).toBeTruthy()
    expect(result.readingId).toBeTruthy()

    // Verify graph schema was created
    const schema = await payload.findByID({ collection: 'graph-schemas', id: result.graphSchemaId })
    expect(schema.name).toBe('CustomerEmail')

    // Verify reading was created
    const reading = await payload.findByID({ collection: 'readings', id: result.readingId })
    expect(reading.text).toBe('Customer has Email')

    // Verify roles were auto-created by the afterChange hook
    const roles = await payload.find({
      collection: 'roles',
      where: { graphSchema: { equals: result.graphSchemaId } },
      sort: 'createdAt',
    })
    expect(roles.docs.length).toBe(2)
  })

  it('skips duplicate readings', async () => {
    const result = await ingestReading(payload, {
      text: 'Customer has Email',
      domainId,
    })

    // Should skip — reading already exists
    expect(result.readingId).toBe('')
    expect(result.graphSchemaId).toBeTruthy()
  })

  it('applies multiplicity constraints', async () => {
    await payload.create({ collection: 'nouns', data: { name: 'Order', objectType: 'entity', domain: domainId } })
    await payload.create({ collection: 'nouns', data: { name: 'Status', objectType: 'value', domain: domainId } })

    const result = await ingestReading(payload, {
      text: 'Order has Status',
      domainId,
      multiplicity: '*:1',
    })

    expect(result.errors).toHaveLength(0)
    expect(result.readingId).toBeTruthy()

    // Verify a constraint was created
    const constraints = await payload.find({
      collection: 'constraints',
      where: { domain: { equals: domainId } },
    })
    expect(constraints.docs.length).toBeGreaterThanOrEqual(1)

    // Verify constraint span was created
    const spans = await payload.find({
      collection: 'constraint-spans',
      where: { domain: { equals: domainId } },
    })
    expect(spans.docs.length).toBeGreaterThanOrEqual(1)
  })
})

describe('ingestClaims', () => {
  let domainId: string

  beforeAll(async () => {
    // Use a fresh domain
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'test-ingest-claims', name: 'Test Ingest Claims' },
    })
    domainId = domain.id
  }, 120_000)

  it('ingests nouns, readings, and constraints from structured claims', async () => {
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Project', objectType: 'entity', plural: 'projects' },
        { name: 'ProjectName', objectType: 'value', valueType: 'string' },
      ],
      readings: [
        {
          text: 'Project has ProjectName',
          nouns: ['Project', 'ProjectName'],
          predicate: 'has',
          multiplicity: '*:1',
        },
      ],
      constraints: [
        {
          kind: 'UC',
          modality: 'Alethic',
          reading: 'Project has ProjectName',
          roles: [0],
        },
      ],
    }

    const result = await ingestClaims(payload, { claims, domainId })

    expect(result.errors).toHaveLength(0)
    expect(result.nouns).toBe(2)
    expect(result.readings).toBe(1)

    // Verify nouns were created
    const nouns = await payload.find({
      collection: 'nouns',
      where: { domain: { equals: domainId } },
    })
    expect(nouns.docs.length).toBe(2)
    const projectNoun = nouns.docs.find((n: any) => n.name === 'Project')
    expect(projectNoun).toBeTruthy()
    expect(projectNoun.objectType).toBe('entity')
    const nameNoun = nouns.docs.find((n: any) => n.name === 'ProjectName')
    expect(nameNoun).toBeTruthy()
    expect(nameNoun.objectType).toBe('value')

    // Verify reading was created
    const readings = await payload.find({
      collection: 'readings',
      where: { domain: { equals: domainId } },
    })
    expect(readings.docs.length).toBe(1)
    expect((readings.docs[0] as any).text).toBe('Project has ProjectName')

    // Verify graph schema was created
    const schemas = await payload.find({
      collection: 'graph-schemas',
      where: { domain: { equals: domainId } },
    })
    expect(schemas.docs.length).toBe(1)
    expect((schemas.docs[0] as any).name).toBe('ProjectProjectName')

    // Verify roles were auto-created by the hook
    const roles = await payload.find({
      collection: 'roles',
      where: { graphSchema: { equals: schemas.docs[0].id } },
      sort: 'createdAt',
    })
    expect(roles.docs.length).toBe(2)

    // Verify the explicit constraint was created (from claims.constraints)
    const constraints = await payload.find({
      collection: 'constraints',
      where: { domain: { equals: domainId } },
    })
    // At least the explicit constraint + the multiplicity-derived one
    expect(constraints.docs.length).toBeGreaterThanOrEqual(1)
  })

  it('handles subtypes', async () => {
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'test-claims-subtypes', name: 'Test Claims Subtypes' },
    })

    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Animal', objectType: 'entity' },
        { name: 'Dog', objectType: 'entity' },
      ],
      readings: [],
      constraints: [],
      subtypes: [{ child: 'Dog', parent: 'Animal' }],
    }

    const result = await ingestClaims(payload, { claims, domainId: domain.id })

    expect(result.errors).toHaveLength(0)
    expect(result.nouns).toBe(2)

    // Verify superType was set
    const dog = await payload.find({
      collection: 'nouns',
      where: { name: { equals: 'Dog' }, domain: { equals: domain.id } },
      limit: 1,
      depth: 1,
    })
    const superType = dog.docs[0].superType
    const superTypeName = typeof superType === 'string' ? null : superType?.name
    expect(superTypeName).toBe('Animal')
  })

  it('handles state machine transitions', async () => {
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'test-claims-sm', name: 'Test Claims SM' },
    })

    const claims: ExtractedClaims = {
      nouns: [{ name: 'Ticket', objectType: 'entity' }],
      readings: [],
      constraints: [],
      transitions: [
        { entity: 'Ticket', from: 'Open', to: 'InProgress', event: 'start' },
        { entity: 'Ticket', from: 'InProgress', to: 'Closed', event: 'close' },
      ],
    }

    const result = await ingestClaims(payload, { claims, domainId: domain.id })

    expect(result.errors).toHaveLength(0)
    expect(result.stateMachines).toBe(1)

    // Verify state machine definition was created
    const defs = await payload.find({
      collection: 'state-machine-definitions',
      where: { domain: { equals: domain.id } },
    })
    expect(defs.docs.length).toBe(1)

    // Verify statuses
    const statuses = await payload.find({
      collection: 'statuses',
      where: { stateMachineDefinition: { equals: defs.docs[0].id } },
    })
    expect(statuses.docs.length).toBe(3) // Open, InProgress, Closed

    // Verify transitions — use depth: 0 to get raw IDs for filtering
    const transitions = await payload.find({ collection: 'transitions', pagination: false, depth: 0 })
    // Filter transitions relevant to our statuses
    const statusIds = new Set(statuses.docs.map((s: any) => s.id))
    const relevantTransitions = transitions.docs.filter(
      (t: any) => statusIds.has(t.from) || statusIds.has(t.to),
    )
    expect(relevantTransitions.length).toBe(2)
  })

  it('is idempotent for existing readings', async () => {
    // Re-ingest the same claims to the first domain
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Project', objectType: 'entity', plural: 'projects' },
        { name: 'ProjectName', objectType: 'value', valueType: 'string' },
      ],
      readings: [
        {
          text: 'Project has ProjectName',
          nouns: ['Project', 'ProjectName'],
          predicate: 'has',
        },
      ],
      constraints: [],
    }

    const result = await ingestClaims(payload, { claims, domainId })

    expect(result.errors).toHaveLength(0)
    expect(result.skipped).toBe(1) // reading already exists
    expect(result.readings).toBe(0) // nothing new created
  })
})
