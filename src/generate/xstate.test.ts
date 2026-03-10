import { describe, it, expect } from 'vitest'
import { generateXState } from './xstate'

// ---------------------------------------------------------------------------
// Mock DB helper
// ---------------------------------------------------------------------------

function mockDB(data: Record<string, any[]>) {
  return {
    findInCollection: async (slug: string, where?: any, _opts?: any) => {
      let docs = data[slug] || []
      if (where) {
        docs = docs.filter((doc: any) => {
          for (const [key, condition] of Object.entries(where)) {
            const cond = condition as any
            if (cond?.equals !== undefined) {
              if (doc[key] !== cond.equals) return false
            }
          }
          return true
        })
      }
      return { docs, totalDocs: docs.length, hasNextPage: false, page: 1, limit: 100 }
    },
  }
}

// ---------------------------------------------------------------------------
// Test data factories
// ---------------------------------------------------------------------------

let autoId = 0
function uid(): string {
  return `id-${++autoId}`
}

function mkNoun(id: string, name: string) {
  return { id, name, objectType: 'entity', domain: 'dom-1' }
}

function mkSmDef(id: string, nounId: string) {
  return { id, noun: nounId, domain: 'dom-1' }
}

function mkStatus(id: string, smDefId: string, name: string, createdAt?: string) {
  return { id, stateMachineDefinition: smDefId, name, createdAt: createdAt || new Date().toISOString() }
}

function mkTransition(id: string, fromId: string, toId: string, eventTypeId: string, verbId?: string) {
  return { id, from: fromId, to: toId, eventType: eventTypeId, verb: verbId }
}

function mkEventType(id: string, name: string) {
  return { id, name }
}

function mkVerb(id: string, funcId: string) {
  return { id, function: funcId }
}

function mkFunc(id: string, callbackUrl: string, httpMethod?: string) {
  return { id, callbackUrl, httpMethod: httpMethod || 'POST' }
}

function mkRole(id: string, nounId: string, graphSchemaId: string) {
  return { id, noun: nounId, graphSchema: graphSchemaId }
}

function mkReading(id: string, text: string, graphSchemaId: string) {
  return { id, text, graphSchema: graphSchemaId }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('generateXState', () => {
  beforeEach(() => {
    autoId = 0
  })

  // -------------------------------------------------------------------------
  // Empty domain
  // -------------------------------------------------------------------------
  it('returns empty files for domain with no state machines', async () => {
    const db = mockDB({
      'state-machine-definitions': [],
      nouns: [],
      statuses: [],
      transitions: [],
      'event-types': [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    expect(result.files).toEqual({})
  })

  // -------------------------------------------------------------------------
  // Machine with 2 states and 1 transition
  // -------------------------------------------------------------------------
  it('generates correct XState config for 2 states and 1 transition', async () => {
    const noun = mkNoun('n1', 'SupportRequest')
    const smDef = mkSmDef('sm1', 'n1')
    const s1 = mkStatus('s1', 'sm1', 'New', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Open', '2025-01-02T00:00:00Z')
    const et = mkEventType('et1', 'OPEN')
    const tr = mkTransition('t1', 's1', 's2', 'et1')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [s1, s2],
      transitions: [tr],
      'event-types': [et],
      verbs: [],
      functions: [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')

    // XState config
    const config = JSON.parse(result.files['state-machines/support-request.json'])
    expect(config.id).toBe('support-request')
    expect(config.initial).toBe('New')
    expect(config.states).toEqual({
      New: { on: { OPEN: { target: 'Open' } } },
      Open: {},
    })
  })

  // -------------------------------------------------------------------------
  // Initial state detection (no incoming transitions)
  // -------------------------------------------------------------------------
  it('picks the status with no incoming transitions as initial state', async () => {
    const noun = mkNoun('n1', 'Order')
    const smDef = mkSmDef('sm1', 'n1')
    // Three statuses: Pending, Confirmed, Shipped
    // Transitions: Pending→Confirmed, Confirmed→Shipped
    // Pending has no incoming → it should be initial
    const s1 = mkStatus('s1', 'sm1', 'Pending', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Confirmed', '2025-01-02T00:00:00Z')
    const s3 = mkStatus('s3', 'sm1', 'Shipped', '2025-01-03T00:00:00Z')
    const et1 = mkEventType('et1', 'CONFIRM')
    const et2 = mkEventType('et2', 'SHIP')
    const tr1 = mkTransition('t1', 's1', 's2', 'et1')
    const tr2 = mkTransition('t2', 's2', 's3', 'et2')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [s1, s2, s3],
      transitions: [tr1, tr2],
      'event-types': [et1, et2],
      verbs: [],
      functions: [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    const config = JSON.parse(result.files['state-machines/order.json'])

    expect(config.initial).toBe('Pending')
    expect(config.states.Pending.on.CONFIRM.target).toBe('Confirmed')
    expect(config.states.Confirmed.on.SHIP.target).toBe('Shipped')
    expect(config.states.Shipped).toEqual({})
  })

  // -------------------------------------------------------------------------
  // Falls back to first status when all have incoming
  // -------------------------------------------------------------------------
  it('falls back to first status when all have incoming transitions', async () => {
    const noun = mkNoun('n1', 'Toggle')
    const smDef = mkSmDef('sm1', 'n1')
    // Circular: On→Off, Off→On — both have incoming
    const s1 = mkStatus('s1', 'sm1', 'On', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Off', '2025-01-02T00:00:00Z')
    const et1 = mkEventType('et1', 'TURN_OFF')
    const et2 = mkEventType('et2', 'TURN_ON')
    const tr1 = mkTransition('t1', 's1', 's2', 'et1')
    const tr2 = mkTransition('t2', 's2', 's1', 'et2')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [s1, s2],
      transitions: [tr1, tr2],
      'event-types': [et1, et2],
      verbs: [],
      functions: [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    const config = JSON.parse(result.files['state-machines/toggle.json'])

    // Both have incoming, so first by createdAt wins
    expect(config.initial).toBe('On')
  })

  // -------------------------------------------------------------------------
  // Agent tools generated from unique events
  // -------------------------------------------------------------------------
  it('generates agent tools from unique events', async () => {
    const noun = mkNoun('n1', 'SupportRequest')
    const smDef = mkSmDef('sm1', 'n1')
    const s1 = mkStatus('s1', 'sm1', 'New', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Open', '2025-01-02T00:00:00Z')
    const s3 = mkStatus('s3', 'sm1', 'Closed', '2025-01-03T00:00:00Z')
    const et1 = mkEventType('et1', 'OPEN')
    const et2 = mkEventType('et2', 'CLOSE')
    const tr1 = mkTransition('t1', 's1', 's2', 'et1')
    const tr2 = mkTransition('t2', 's2', 's3', 'et2')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [s1, s2, s3],
      transitions: [tr1, tr2],
      'event-types': [et1, et2],
      verbs: [],
      functions: [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    const tools = JSON.parse(result.files['agents/support-request-tools.json'])

    expect(tools).toHaveLength(2)
    expect(tools[0]).toEqual({
      name: 'OPEN',
      description: 'Transition from New to Open',
      parameters: { type: 'object', properties: {} },
    })
    expect(tools[1]).toEqual({
      name: 'CLOSE',
      description: 'Transition from Open to Closed',
      parameters: { type: 'object', properties: {} },
    })
  })

  // -------------------------------------------------------------------------
  // Deduplication for events used from multiple sources
  // -------------------------------------------------------------------------
  it('deduplicates events that appear in multiple transitions', async () => {
    const noun = mkNoun('n1', 'Task')
    const smDef = mkSmDef('sm1', 'n1')
    const s1 = mkStatus('s1', 'sm1', 'Draft', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Active', '2025-01-02T00:00:00Z')
    const s3 = mkStatus('s3', 'sm1', 'Archived', '2025-01-03T00:00:00Z')
    // ARCHIVE event can happen from both Draft and Active
    const et1 = mkEventType('et1', 'ACTIVATE')
    const et2 = mkEventType('et2', 'ARCHIVE')
    const tr1 = mkTransition('t1', 's1', 's2', 'et1')
    const tr2 = mkTransition('t2', 's2', 's3', 'et2')
    const tr3 = mkTransition('t3', 's1', 's3', 'et2')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [s1, s2, s3],
      transitions: [tr1, tr2, tr3],
      'event-types': [et1, et2],
      verbs: [],
      functions: [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    const tools = JSON.parse(result.files['agents/task-tools.json'])

    expect(tools).toHaveLength(2)
    const archiveTool = tools.find((t: any) => t.name === 'ARCHIVE')
    expect(archiveTool.description).toBe('Transition from Draft or Active to Archived')
  })

  // -------------------------------------------------------------------------
  // System prompt includes readings and state names
  // -------------------------------------------------------------------------
  it('generates system prompt with readings and state info', async () => {
    const noun = mkNoun('n1', 'SupportRequest')
    const customerNoun = mkNoun('n2', 'Customer')
    const smDef = mkSmDef('sm1', 'n1')
    const s1 = mkStatus('s1', 'sm1', 'New', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Open', '2025-01-02T00:00:00Z')
    const et = mkEventType('et1', 'OPEN')
    const tr = mkTransition('t1', 's1', 's2', 'et1')

    // Roles connecting SupportRequest to a graph schema
    const role1 = mkRole('r1', 'n1', 'gs1')
    const role2 = mkRole('r2', 'n2', 'gs1')
    const reading1 = mkReading('rd1', 'Customer submits SupportRequest', 'gs1')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun, customerNoun],
      statuses: [s1, s2],
      transitions: [tr],
      'event-types': [et],
      verbs: [],
      functions: [],
      roles: [role1, role2],
      readings: [reading1],
    })

    const result = await generateXState(db, 'dom-1')
    const prompt = result.files['agents/support-request-prompt.md']

    expect(prompt).toContain('# SupportRequest Agent')
    expect(prompt).toContain('## Domain Model')
    expect(prompt).toContain('- Customer submits SupportRequest')
    expect(prompt).toContain('States: New, Open')
    expect(prompt).toContain('- **OPEN**: New → Open')
    expect(prompt).toContain('## Current State: {{currentState}}')
    expect(prompt).toContain('You operate within the domain model above.')
  })

  // -------------------------------------------------------------------------
  // Prompt expands to related schemas (one level)
  // -------------------------------------------------------------------------
  it('expands readings to related schemas via noun graph', async () => {
    const noun = mkNoun('n1', 'Order')
    const customerNoun = mkNoun('n2', 'Customer')
    const addressNoun = mkNoun('n3', 'Address')
    const smDef = mkSmDef('sm1', 'n1')
    const s1 = mkStatus('s1', 'sm1', 'Pending', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Confirmed', '2025-01-02T00:00:00Z')
    const et = mkEventType('et1', 'CONFIRM')
    const tr = mkTransition('t1', 's1', 's2', 'et1')

    // gs1: Order + Customer (direct schema for Order)
    // gs2: Customer + Address (related via Customer, expanded one level)
    const role1 = mkRole('r1', 'n1', 'gs1')
    const role2 = mkRole('r2', 'n2', 'gs1')
    const role3 = mkRole('r3', 'n2', 'gs2')
    const role4 = mkRole('r4', 'n3', 'gs2')
    const reading1 = mkReading('rd1', 'Customer places Order', 'gs1')
    const reading2 = mkReading('rd2', 'Customer has Address', 'gs2')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun, customerNoun, addressNoun],
      statuses: [s1, s2],
      transitions: [tr],
      'event-types': [et],
      verbs: [],
      functions: [],
      roles: [role1, role2, role3, role4],
      readings: [reading1, reading2],
    })

    const result = await generateXState(db, 'dom-1')
    const prompt = result.files['agents/order-prompt.md']

    // Both readings included because Customer connects to both schemas
    expect(prompt).toContain('- Customer places Order')
    expect(prompt).toContain('- Customer has Address')
  })

  // -------------------------------------------------------------------------
  // Machine with verb→function callback
  // -------------------------------------------------------------------------
  it('includes callback metadata when transition has verb→function', async () => {
    const noun = mkNoun('n1', 'Payment')
    const smDef = mkSmDef('sm1', 'n1')
    const s1 = mkStatus('s1', 'sm1', 'Pending', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Processed', '2025-01-02T00:00:00Z')
    const et = mkEventType('et1', 'PROCESS')
    const verb = mkVerb('v1', 'f1')
    const func = mkFunc('f1', 'https://api.example.com/process', 'POST')
    const tr = mkTransition('t1', 's1', 's2', 'et1', 'v1')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [s1, s2],
      transitions: [tr],
      'event-types': [et],
      verbs: [verb],
      functions: [func],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    const config = JSON.parse(result.files['state-machines/payment.json'])

    expect(config.states.Pending.on.PROCESS).toEqual({
      target: 'Processed',
      meta: {
        callback: { url: 'https://api.example.com/process', method: 'POST' },
      },
    })
  })

  // -------------------------------------------------------------------------
  // Callback defaults to POST when httpMethod missing
  // -------------------------------------------------------------------------
  it('defaults callback method to POST when httpMethod is not set', async () => {
    const noun = mkNoun('n1', 'Invoice')
    const smDef = mkSmDef('sm1', 'n1')
    const s1 = mkStatus('s1', 'sm1', 'Draft', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Sent', '2025-01-02T00:00:00Z')
    const et = mkEventType('et1', 'SEND')
    const verb = mkVerb('v1', 'f1')
    const func = { id: 'f1', callbackUrl: 'https://api.example.com/send' } // no httpMethod
    const tr = mkTransition('t1', 's1', 's2', 'et1', 'v1')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [s1, s2],
      transitions: [tr],
      'event-types': [et],
      verbs: [verb],
      functions: [func],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    const config = JSON.parse(result.files['state-machines/invoice.json'])
    expect(config.states.Draft.on.SEND.meta.callback.method).toBe('POST')
  })

  // -------------------------------------------------------------------------
  // Multiple machines in one domain
  // -------------------------------------------------------------------------
  it('generates files for multiple machines in the same domain', async () => {
    const noun1 = mkNoun('n1', 'Order')
    const noun2 = mkNoun('n2', 'Payment')
    const smDef1 = mkSmDef('sm1', 'n1')
    const smDef2 = mkSmDef('sm2', 'n2')

    const s1a = mkStatus('s1a', 'sm1', 'Pending', '2025-01-01T00:00:00Z')
    const s1b = mkStatus('s1b', 'sm1', 'Confirmed', '2025-01-02T00:00:00Z')
    const s2a = mkStatus('s2a', 'sm2', 'Unpaid', '2025-01-01T00:00:00Z')
    const s2b = mkStatus('s2b', 'sm2', 'Paid', '2025-01-02T00:00:00Z')

    const et1 = mkEventType('et1', 'CONFIRM')
    const et2 = mkEventType('et2', 'PAY')

    const tr1 = mkTransition('t1', 's1a', 's1b', 'et1')
    const tr2 = mkTransition('t2', 's2a', 's2b', 'et2')

    const db = mockDB({
      'state-machine-definitions': [smDef1, smDef2],
      nouns: [noun1, noun2],
      statuses: [s1a, s1b, s2a, s2b],
      transitions: [tr1, tr2],
      'event-types': [et1, et2],
      verbs: [],
      functions: [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')

    // Should have files for both machines
    expect(result.files['state-machines/order.json']).toBeDefined()
    expect(result.files['state-machines/payment.json']).toBeDefined()
    expect(result.files['agents/order-tools.json']).toBeDefined()
    expect(result.files['agents/payment-tools.json']).toBeDefined()
    expect(result.files['agents/order-prompt.md']).toBeDefined()
    expect(result.files['agents/payment-prompt.md']).toBeDefined()

    const orderConfig = JSON.parse(result.files['state-machines/order.json'])
    expect(orderConfig.id).toBe('order')
    expect(orderConfig.initial).toBe('Pending')

    const paymentConfig = JSON.parse(result.files['state-machines/payment.json'])
    expect(paymentConfig.id).toBe('payment')
    expect(paymentConfig.initial).toBe('Unpaid')
  })

  // -------------------------------------------------------------------------
  // Skips state machine definitions with no statuses
  // -------------------------------------------------------------------------
  it('skips state machine definitions that have no statuses', async () => {
    const noun = mkNoun('n1', 'Widget')
    const smDef = mkSmDef('sm1', 'n1')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [], // no statuses
      transitions: [],
      'event-types': [],
      verbs: [],
      functions: [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    expect(result.files).toEqual({})
  })

  // -------------------------------------------------------------------------
  // Machine name kebab-case conversion
  // -------------------------------------------------------------------------
  it('converts PascalCase noun names to kebab-case for machine names', async () => {
    const noun = mkNoun('n1', 'MyLongEntityName')
    const smDef = mkSmDef('sm1', 'n1')
    const s1 = mkStatus('s1', 'sm1', 'Active', '2025-01-01T00:00:00Z')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [s1],
      transitions: [],
      'event-types': [],
      verbs: [],
      functions: [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    expect(result.files['state-machines/my-long-entity-name.json']).toBeDefined()
  })

  // -------------------------------------------------------------------------
  // Unknown noun falls back to 'unknown'
  // -------------------------------------------------------------------------
  it('uses "unknown" as machine name when noun is not found', async () => {
    const smDef = mkSmDef('sm1', 'n-missing')
    const s1 = mkStatus('s1', 'sm1', 'Active', '2025-01-01T00:00:00Z')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [], // no nouns
      statuses: [s1],
      transitions: [],
      'event-types': [],
      verbs: [],
      functions: [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    expect(result.files['state-machines/unknown.json']).toBeDefined()

    const prompt = result.files['agents/unknown-prompt.md']
    expect(prompt).toContain('# Agent Agent')
  })

  // -------------------------------------------------------------------------
  // Prompt with no readings still produces valid structure
  // -------------------------------------------------------------------------
  it('generates prompt even when no readings match', async () => {
    const noun = mkNoun('n1', 'Ticket')
    const smDef = mkSmDef('sm1', 'n1')
    const s1 = mkStatus('s1', 'sm1', 'Open', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Closed', '2025-01-02T00:00:00Z')
    const et = mkEventType('et1', 'CLOSE')
    const tr = mkTransition('t1', 's1', 's2', 'et1')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [s1, s2],
      transitions: [tr],
      'event-types': [et],
      verbs: [],
      functions: [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    const prompt = result.files['agents/ticket-prompt.md']

    expect(prompt).toContain('# Ticket Agent')
    expect(prompt).toContain('## Domain Model')
    expect(prompt).toContain('States: Open, Closed')
    expect(prompt).toContain('- **CLOSE**: Open → Closed')
  })

  // -------------------------------------------------------------------------
  // Verb without function does not produce callback
  // -------------------------------------------------------------------------
  it('does not add callback when verb has no function', async () => {
    const noun = mkNoun('n1', 'Job')
    const smDef = mkSmDef('sm1', 'n1')
    const s1 = mkStatus('s1', 'sm1', 'Queued', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Running', '2025-01-02T00:00:00Z')
    const et = mkEventType('et1', 'START')
    const verb = { id: 'v1' } // verb with no function field
    const tr = mkTransition('t1', 's1', 's2', 'et1', 'v1')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [s1, s2],
      transitions: [tr],
      'event-types': [et],
      verbs: [verb],
      functions: [],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    const config = JSON.parse(result.files['state-machines/job.json'])

    expect(config.states.Queued.on.START).toEqual({ target: 'Running' })
    expect(config.states.Queued.on.START.meta).toBeUndefined()
  })

  // -------------------------------------------------------------------------
  // Function without callbackUrl does not produce callback
  // -------------------------------------------------------------------------
  it('does not add callback when function has no callbackUrl', async () => {
    const noun = mkNoun('n1', 'Task')
    const smDef = mkSmDef('sm1', 'n1')
    const s1 = mkStatus('s1', 'sm1', 'Todo', '2025-01-01T00:00:00Z')
    const s2 = mkStatus('s2', 'sm1', 'Done', '2025-01-02T00:00:00Z')
    const et = mkEventType('et1', 'COMPLETE')
    const verb = mkVerb('v1', 'f1')
    const func = { id: 'f1' } // no callbackUrl
    const tr = mkTransition('t1', 's1', 's2', 'et1', 'v1')

    const db = mockDB({
      'state-machine-definitions': [smDef],
      nouns: [noun],
      statuses: [s1, s2],
      transitions: [tr],
      'event-types': [et],
      verbs: [verb],
      functions: [func],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'dom-1')
    const config = JSON.parse(result.files['state-machines/task.json'])

    expect(config.states.Todo.on.COMPLETE).toEqual({ target: 'Done' })
    expect(config.states.Todo.on.COMPLETE.meta).toBeUndefined()
  })
})
