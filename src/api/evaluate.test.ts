import { describe, it, expect, vi } from 'vitest'

// Mock the WASM binary import — use the resolved path from evaluate.ts's perspective
vi.mock(
  '../../crates/fol-engine/pkg/fol_engine_bg.wasm',
  () => ({ default: {} }),
)

// Mock the WASM JS bindings
vi.mock(
  '../../crates/fol-engine/pkg/fol_engine.js',
  () => ({
    initSync: vi.fn(() => { throw new Error('WASM not available in test') }),
    load_ir: vi.fn(),
    evaluate_response: vi.fn(),
    synthesize_noun: vi.fn(),
  }),
)

import { handleEvaluate, handleSynthesize } from './evaluate'

function makeRequest(method: string, body?: any, path = '/api/evaluate'): Request {
  return new Request(`http://localhost${path}`, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body ? JSON.stringify(body) : undefined,
  })
}

function makeMockEnv(docs: any[] = []) {
  return {
    DOMAIN_DB: {
      idFromName: vi.fn().mockReturnValue('test-id'),
      get: vi.fn().mockReturnValue({
        findInCollection: vi.fn().mockResolvedValue({ docs }),
      }),
    },
    ENVIRONMENT: 'test',
  } as any
}

describe('evaluate endpoint', () => {
  it('exports handleEvaluate', async () => {
    expect(handleEvaluate).toBeDefined()
    expect(typeof handleEvaluate).toBe('function')
  })

  it('returns 405 for non-POST methods', async () => {
    const req = makeRequest('GET')
    const res = await handleEvaluate(req, makeMockEnv())
    expect(res.status).toBe(405)
  })

  it('returns 400 when domainId is missing', async () => {
    const req = makeRequest('POST', { response: { text: 'hello' } })
    const res = await handleEvaluate(req, makeMockEnv())
    expect(res.status).toBe(400)
    const body = await res.json() as any
    expect(body.errors[0].message).toContain('domainId')
  })

  it('returns 400 when response.text is missing', async () => {
    const req = makeRequest('POST', { domainId: 'd1' })
    const res = await handleEvaluate(req, makeMockEnv())
    expect(res.status).toBe(400)
    const body = await res.json() as any
    expect(body.errors[0].message).toContain('response.text')
  })

  it('returns warning when no domain schema exists', async () => {
    const req = makeRequest('POST', { domainId: 'd1', response: { text: 'hello' } })
    const res = await handleEvaluate(req, makeMockEnv([]))
    expect(res.status).toBe(200)
    const body = await res.json() as any
    expect(body.violations).toEqual([])
    expect(body.warning).toContain('No domain schema')
    expect(body.domainId).toBe('d1')
  })

  it('returns WASM unavailable warning when IR exists but WASM fails', async () => {
    const irDoc = { output: JSON.stringify({ domain: 'd1', constraints: [{ id: 'c1' }] }) }
    const req = makeRequest('POST', { domainId: 'd1', response: { text: 'hello' } })
    const res = await handleEvaluate(req, makeMockEnv([irDoc]))
    expect(res.status).toBe(200)
    const body = await res.json() as any
    expect(body.violations).toEqual([])
    expect(body.constraintCount).toBe(1)
    expect(body.warning).toContain('WASM evaluator not available')
  })
})

// ── Mock domain schema for synthesize tests ──────────────────────────
const mockDomainSchema = {
  domain: 'university',
  nouns: {
    Student: { worldAssumption: 'closed' },
    Course: { worldAssumption: 'open' },
    Grade: { worldAssumption: 'closed' },
  },
  factTypes: {
    'ft-enroll': {
      reading: 'Student is enrolled in Course',
      roles: [
        { nounName: 'Student', roleName: 'enrollee' },
        { nounName: 'Course', roleName: 'course' },
      ],
    },
    'ft-grade': {
      reading: 'Student received Grade for Course',
      roles: [
        { nounName: 'Student', roleName: 'student' },
        { nounName: 'Grade', roleName: 'grade' },
        { nounName: 'Course', roleName: 'course' },
      ],
    },
  },
  constraints: [
    {
      id: 'uc-enroll',
      text: 'Each Student is enrolled in at most one Course',
      kind: 'uniqueness',
      modality: 'alethic',
      deonticOperator: null,
      spans: [{ factTypeId: 'ft-enroll', roleIndex: 0 }],
    },
    {
      id: 'mc-grade',
      text: 'Each Student received some Grade for each Course',
      kind: 'mandatory',
      modality: 'alethic',
      deonticOperator: null,
      spans: [{ factTypeId: 'ft-grade', roleIndex: 0 }],
    },
    {
      id: 'uc-other',
      text: 'Constraint not spanning Student fact types',
      kind: 'uniqueness',
      modality: 'alethic',
      deonticOperator: null,
      spans: [{ factTypeId: 'ft-other', roleIndex: 0 }],
    },
  ],
  stateMachines: {
    'sm-student': { nounName: 'Student', states: ['active', 'graduated'] },
  },
  derivationRules: [
    {
      id: 'derive-subtype-Student',
      antecedentFactTypeIds: [],
      consequent: 'Student is a subtype',
    },
    {
      id: 'dr-enroll',
      antecedentFactTypeIds: ['ft-enroll'],
      consequent: 'derived enrollment fact',
    },
    {
      id: 'dr-unrelated',
      antecedentFactTypeIds: ['ft-other'],
      consequent: 'unrelated derivation',
    },
  ],
}

function makeSynthRequest(method: string, body?: any): Request {
  return makeRequest(method, body, '/api/synthesize')
}

describe('handleSynthesize', () => {
  it('returns 405 for non-POST methods', async () => {
    const req = makeSynthRequest('GET')
    const res = await handleSynthesize(req, makeMockEnv())
    expect(res.status).toBe(405)
  })

  it('returns 400 when domainId is missing', async () => {
    const req = makeSynthRequest('POST', { nounName: 'Student' })
    const res = await handleSynthesize(req, makeMockEnv())
    expect(res.status).toBe(400)
    const body = await res.json() as any
    expect(body.errors[0].message).toContain('domainId and nounName are required')
  })

  it('returns 400 when nounName is missing', async () => {
    const req = makeSynthRequest('POST', { domainId: 'd1' })
    const res = await handleSynthesize(req, makeMockEnv())
    expect(res.status).toBe(400)
    const body = await res.json() as any
    expect(body.errors[0].message).toContain('domainId and nounName are required')
  })

  it('returns error when no domain schema exists', async () => {
    const req = makeSynthRequest('POST', { domainId: 'd1', nounName: 'Student' })
    const res = await handleSynthesize(req, makeMockEnv([]))
    expect(res.status).toBe(200)
    const body = await res.json() as any
    expect(body.error).toContain('No domain schema')
    expect(body.suggestion).toBeDefined()
  })

  it('returns error when noun is not found in domain schema', async () => {
    const irDoc = { output: JSON.stringify(mockDomainSchema) }
    const req = makeSynthRequest('POST', { domainId: 'university', nounName: 'Professor' })
    const res = await handleSynthesize(req, makeMockEnv([irDoc]))
    expect(res.status).toBe(200)
    const body = await res.json() as any
    expect(body.error).toContain("Noun 'Professor' not found")
  })

  it('synthesizes knowledge for a noun via fallback (WASM unavailable)', async () => {
    const irDoc = { output: JSON.stringify(mockDomainSchema) }
    const req = makeSynthRequest('POST', { domainId: 'university', nounName: 'Student' })
    const res = await handleSynthesize(req, makeMockEnv([irDoc]))
    expect(res.status).toBe(200)
    const body = await res.json() as any

    // Verify top-level shape
    expect(body.nounName).toBe('Student')
    expect(body.worldAssumption).toBe('closed')

    // Verify participating fact types
    expect(body.participatesIn).toHaveLength(2)
    const ftIds = body.participatesIn.map((p: any) => p.id)
    expect(ftIds).toContain('ft-enroll')
    expect(ftIds).toContain('ft-grade')

    // Verify roleIndex is correct for Student in each fact type
    const enrollEntry = body.participatesIn.find((p: any) => p.id === 'ft-enroll')
    expect(enrollEntry.roleIndex).toBe(0)
    expect(enrollEntry.reading).toBe('Student is enrolled in Course')

    const gradeEntry = body.participatesIn.find((p: any) => p.id === 'ft-grade')
    expect(gradeEntry.roleIndex).toBe(0)
    expect(gradeEntry.reading).toBe('Student received Grade for Course')

    // Verify applicable constraints (only those spanning Student fact types)
    expect(body.applicableConstraints).toHaveLength(2)
    const constraintIds = body.applicableConstraints.map((c: any) => c.id)
    expect(constraintIds).toContain('uc-enroll')
    expect(constraintIds).toContain('mc-grade')
    // uc-other should NOT be included (spans ft-other, not Student)
    expect(constraintIds).not.toContain('uc-other')

    // Verify constraint shape
    const ucEnroll = body.applicableConstraints.find((c: any) => c.id === 'uc-enroll')
    expect(ucEnroll.kind).toBe('uniqueness')
    expect(ucEnroll.modality).toBe('alethic')
    expect(ucEnroll.text).toBe('Each Student is enrolled in at most one Course')

    // Verify state machines
    expect(body.stateMachines).toHaveLength(1)
    expect(body.stateMachines[0].nounName).toBe('Student')

    // Verify derivation rules (subtype rule + enrollment rule, not unrelated)
    expect(body.derivationRules).toHaveLength(2)
    const drIds = body.derivationRules.map((dr: any) => dr.id)
    expect(drIds).toContain('derive-subtype-Student')
    expect(drIds).toContain('dr-enroll')
    expect(drIds).not.toContain('dr-unrelated')

    // Verify derived facts (empty in fallback)
    expect(body.derivedFacts).toEqual([])

    // Verify related nouns
    expect(body.relatedNouns.length).toBeGreaterThan(0)
    const relatedNames = body.relatedNouns.map((rn: any) => rn.name)
    expect(relatedNames).toContain('Course')
    expect(relatedNames).toContain('Grade')
    expect(relatedNames).not.toContain('Student')

    // Verify related noun shape
    const courseRelation = body.relatedNouns.find((rn: any) => rn.name === 'Course' && rn.viaFactType === 'ft-enroll')
    expect(courseRelation.viaReading).toBe('Student is enrolled in Course')
    expect(courseRelation.worldAssumption).toBe('open')
  })

  it('synthesizes knowledge for a noun with no constraints or state machines', async () => {
    const minimalSchema = {
      domain: 'test',
      nouns: { Foo: { worldAssumption: 'open' } },
      factTypes: {
        'ft-bar': {
          reading: 'Foo has Bar',
          roles: [{ nounName: 'Foo', roleName: 'foo' }, { nounName: 'Bar', roleName: 'bar' }],
        },
      },
      constraints: [],
      stateMachines: {},
      derivationRules: [],
    }
    const irDoc = { output: JSON.stringify(minimalSchema) }
    const req = makeSynthRequest('POST', { domainId: 'test', nounName: 'Foo' })
    const res = await handleSynthesize(req, makeMockEnv([irDoc]))
    expect(res.status).toBe(200)
    const body = await res.json() as any

    expect(body.nounName).toBe('Foo')
    expect(body.worldAssumption).toBe('open')
    expect(body.participatesIn).toHaveLength(1)
    expect(body.applicableConstraints).toEqual([])
    expect(body.stateMachines).toEqual([])
    expect(body.derivationRules).toEqual([])
    expect(body.derivedFacts).toEqual([])
  })

  it('handles schema stored as pre-parsed object (not string)', async () => {
    const irDoc = { output: mockDomainSchema }
    const req = makeSynthRequest('POST', { domainId: 'university', nounName: 'Course' })
    const res = await handleSynthesize(req, makeMockEnv([irDoc]))
    expect(res.status).toBe(200)
    const body = await res.json() as any

    expect(body.nounName).toBe('Course')
    expect(body.worldAssumption).toBe('open')
    expect(body.participatesIn).toHaveLength(2)
  })

  it('returns 500 when schema output is invalid JSON string', async () => {
    const irDoc = { output: '{not valid json' }
    const req = makeSynthRequest('POST', { domainId: 'd1', nounName: 'Student' })
    const res = await handleSynthesize(req, makeMockEnv([irDoc]))
    expect(res.status).toBe(500)
    const body = await res.json() as any
    expect(body.errors[0].message).toContain('Failed to parse domain schema')
  })
})
