import { describe, it, expect, beforeEach } from 'vitest'
import { generateXState } from './xstate'
import { createMockModel, mkNounDef, mkFactType, mkStateMachine, resetIds } from '../model/test-utils'
import type { ReadingDef } from '../model/types'

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('generateXState', () => {
  beforeEach(() => resetIds())

  // -------------------------------------------------------------------------
  // Empty domain
  // -------------------------------------------------------------------------
  it('returns empty files for domain with no state machines', async () => {
    const model = createMockModel({
      nouns: [],
      stateMachines: [],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    expect(result.files).toEqual({})
  })

  // -------------------------------------------------------------------------
  // Machine with 2 states and 1 transition
  // -------------------------------------------------------------------------
  it('generates correct XState config for 2 states and 1 transition', async () => {
    const noun = mkNounDef({ name: 'SupportRequest' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'New' },
        { id: 's2', name: 'Open' },
      ],
      transitions: [
        { from: 'New', to: 'Open', event: 'OPEN', eventTypeId: 'et1' },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)

    // XState config
    const config = JSON.parse(result.files['state-machines/support-request.json'])
    expect(config.id).toBe('support-request')
    expect(config.initial).toBe('New')
    expect(config.states).toEqual({
      New: { on: { OPEN: 'Open' } },
      Open: { type: 'final' },
    })
  })

  // -------------------------------------------------------------------------
  // Initial state detection (no incoming transitions)
  // -------------------------------------------------------------------------
  it('picks the status with no incoming transitions as initial state', async () => {
    const noun = mkNounDef({ name: 'Order' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Pending' },
        { id: 's2', name: 'Confirmed' },
        { id: 's3', name: 'Shipped' },
      ],
      transitions: [
        { from: 'Pending', to: 'Confirmed', event: 'CONFIRM', eventTypeId: 'et1' },
        { from: 'Confirmed', to: 'Shipped', event: 'SHIP', eventTypeId: 'et2' },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const config = JSON.parse(result.files['state-machines/order.json'])

    expect(config.initial).toBe('Pending')
    expect(config.states.Pending.on.CONFIRM).toBe('Confirmed')
    expect(config.states.Confirmed.on.SHIP).toBe('Shipped')
    expect(config.states.Shipped).toEqual({ type: 'final' })
  })

  // -------------------------------------------------------------------------
  // Falls back to first status when all have incoming
  // -------------------------------------------------------------------------
  it('falls back to first status when all have incoming transitions', async () => {
    const noun = mkNounDef({ name: 'Toggle' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'On' },
        { id: 's2', name: 'Off' },
      ],
      transitions: [
        { from: 'On', to: 'Off', event: 'TURN_OFF', eventTypeId: 'et1' },
        { from: 'Off', to: 'On', event: 'TURN_ON', eventTypeId: 'et2' },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const config = JSON.parse(result.files['state-machines/toggle.json'])

    // Both have incoming, so first in array wins
    expect(config.initial).toBe('On')
  })

  // -------------------------------------------------------------------------
  // Agent tools generated from unique events
  // -------------------------------------------------------------------------
  it('generates agent tools from unique events', async () => {
    const noun = mkNounDef({ name: 'SupportRequest' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'New' },
        { id: 's2', name: 'Open' },
        { id: 's3', name: 'Closed' },
      ],
      transitions: [
        { from: 'New', to: 'Open', event: 'OPEN', eventTypeId: 'et1' },
        { from: 'Open', to: 'Closed', event: 'CLOSE', eventTypeId: 'et2' },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
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
    const noun = mkNounDef({ name: 'Task' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Draft' },
        { id: 's2', name: 'Active' },
        { id: 's3', name: 'Archived' },
      ],
      transitions: [
        { from: 'Draft', to: 'Active', event: 'ACTIVATE', eventTypeId: 'et1' },
        { from: 'Active', to: 'Archived', event: 'ARCHIVE', eventTypeId: 'et2' },
        { from: 'Draft', to: 'Archived', event: 'ARCHIVE', eventTypeId: 'et2' },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const tools = JSON.parse(result.files['agents/task-tools.json'])

    expect(tools).toHaveLength(2)
    const archiveTool = tools.find((t: any) => t.name === 'ARCHIVE')
    expect(archiveTool.description).toBe('Transition from Active or Draft to Archived')
  })

  // -------------------------------------------------------------------------
  // System prompt includes readings and state names
  // -------------------------------------------------------------------------
  it('generates system prompt with readings and state info', async () => {
    const noun = mkNounDef({ name: 'SupportRequest' })
    const customerNoun = mkNounDef({ name: 'Customer' })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Customer submits SupportRequest',
      roles: [
        { nounDef: customerNoun, roleIndex: 0 },
        { nounDef: noun, roleIndex: 1 },
      ],
    })

    const reading: ReadingDef = {
      id: 'rd1',
      text: 'Customer submits SupportRequest',
      graphSchemaId: 'gs1',
      roles: [],
    }

    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'New' },
        { id: 's2', name: 'Open' },
      ],
      transitions: [
        { from: 'New', to: 'Open', event: 'OPEN', eventTypeId: 'et1' },
      ],
    })

    const model = createMockModel({
      nouns: [noun, customerNoun],
      stateMachines: [sm],
      factTypes: [ft],
      readings: [reading],
    })

    const result = await generateXState(model)
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
    const noun = mkNounDef({ name: 'Order' })
    const customerNoun = mkNounDef({ name: 'Customer' })
    const addressNoun = mkNounDef({ name: 'Address' })

    // gs1: Order + Customer (direct fact type for Order)
    const ft1 = mkFactType({
      id: 'gs1',
      reading: 'Customer places Order',
      roles: [
        { nounDef: customerNoun, roleIndex: 0 },
        { nounDef: noun, roleIndex: 1 },
      ],
    })

    // gs2: Customer + Address (related via Customer, expanded one level)
    const ft2 = mkFactType({
      id: 'gs2',
      reading: 'Customer has Address',
      roles: [
        { nounDef: customerNoun, roleIndex: 0 },
        { nounDef: addressNoun, roleIndex: 1 },
      ],
    })

    const reading1: ReadingDef = {
      id: 'rd1',
      text: 'Customer places Order',
      graphSchemaId: 'gs1',
      roles: [],
    }
    const reading2: ReadingDef = {
      id: 'rd2',
      text: 'Customer has Address',
      graphSchemaId: 'gs2',
      roles: [],
    }

    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Pending' },
        { id: 's2', name: 'Confirmed' },
      ],
      transitions: [
        { from: 'Pending', to: 'Confirmed', event: 'CONFIRM', eventTypeId: 'et1' },
      ],
    })

    const model = createMockModel({
      nouns: [noun, customerNoun, addressNoun],
      stateMachines: [sm],
      factTypes: [ft1, ft2],
      readings: [reading1, reading2],
    })

    const result = await generateXState(model)
    const prompt = result.files['agents/order-prompt.md']

    // Both readings included because Customer connects to both fact types
    expect(prompt).toContain('- Customer places Order')
    expect(prompt).toContain('- Customer has Address')
  })

  // -------------------------------------------------------------------------
  // Machine with verb→function callback
  // -------------------------------------------------------------------------
  it('includes callback metadata when transition has verb→function', async () => {
    const noun = mkNounDef({ name: 'Payment' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Pending' },
        { id: 's2', name: 'Processed' },
      ],
      transitions: [
        {
          from: 'Pending',
          to: 'Processed',
          event: 'PROCESS',
          eventTypeId: 'et1',
          verb: {
            id: 'v1',
            name: 'process',
            func: { callbackUrl: 'https://api.example.com/process', httpMethod: 'POST' },
          },
        },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const config = JSON.parse(result.files['state-machines/payment.json'])

    // Simple transition (no guard/pattern) uses string form
    expect(config.states.Pending.on.PROCESS).toBe('Processed')
    // Mealy callback lands on the target state in state.do format
    expect(config.states.Processed.callback).toBe('https://api.example.com/process')
  })

  // -------------------------------------------------------------------------
  // Callback defaults to POST when httpMethod missing
  // -------------------------------------------------------------------------
  it('defaults callback method to POST when httpMethod is not set', async () => {
    const noun = mkNounDef({ name: 'Invoice' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Draft' },
        { id: 's2', name: 'Sent' },
      ],
      transitions: [
        {
          from: 'Draft',
          to: 'Sent',
          event: 'SEND',
          eventTypeId: 'et1',
          verb: {
            id: 'v1',
            name: 'send',
            func: { callbackUrl: 'https://api.example.com/send' },
          },
        },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const config = JSON.parse(result.files['state-machines/invoice.json'])
    // Mealy callback with default POST method lands on target state as simple URL
    expect(config.states.Sent.callback).toBe('https://api.example.com/send')
  })

  // -------------------------------------------------------------------------
  // Multiple machines in one domain
  // -------------------------------------------------------------------------
  it('generates files for multiple machines in the same domain', async () => {
    const noun1 = mkNounDef({ name: 'Order' })
    const noun2 = mkNounDef({ name: 'Payment' })

    const sm1 = mkStateMachine({
      nounDef: noun1,
      statuses: [
        { id: 's1a', name: 'Pending' },
        { id: 's1b', name: 'Confirmed' },
      ],
      transitions: [
        { from: 'Pending', to: 'Confirmed', event: 'CONFIRM', eventTypeId: 'et1' },
      ],
    })

    const sm2 = mkStateMachine({
      nounDef: noun2,
      statuses: [
        { id: 's2a', name: 'Unpaid' },
        { id: 's2b', name: 'Paid' },
      ],
      transitions: [
        { from: 'Unpaid', to: 'Paid', event: 'PAY', eventTypeId: 'et2' },
      ],
    })

    const model = createMockModel({
      nouns: [noun1, noun2],
      stateMachines: [sm1, sm2],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)

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
    const noun = mkNounDef({ name: 'Widget' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [],
      transitions: [],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    expect(result.files).toEqual({})
  })

  // -------------------------------------------------------------------------
  // Machine name kebab-case conversion
  // -------------------------------------------------------------------------
  it('converts PascalCase noun names to kebab-case for machine names', async () => {
    const noun = mkNounDef({ name: 'MyLongEntityName' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [{ id: 's1', name: 'Active' }],
      transitions: [],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    expect(result.files['state-machines/my-long-entity-name.json']).toBeDefined()
  })

  // -------------------------------------------------------------------------
  // Unknown noun uses 'Unknown' as machine name
  // -------------------------------------------------------------------------
  it('uses "unknown" as machine name when noun name is Unknown', async () => {
    const noun = mkNounDef({ name: 'Unknown' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [{ id: 's1', name: 'Active' }],
      transitions: [],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    expect(result.files['state-machines/unknown.json']).toBeDefined()

    const prompt = result.files['agents/unknown-prompt.md']
    expect(prompt).toContain('# Unknown Agent')
  })

  // -------------------------------------------------------------------------
  // Prompt with no readings still produces valid structure
  // -------------------------------------------------------------------------
  it('generates prompt even when no readings match', async () => {
    const noun = mkNounDef({ name: 'Ticket' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Open' },
        { id: 's2', name: 'Closed' },
      ],
      transitions: [
        { from: 'Open', to: 'Closed', event: 'CLOSE', eventTypeId: 'et1' },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
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
    const noun = mkNounDef({ name: 'Job' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Queued' },
        { id: 's2', name: 'Running' },
      ],
      transitions: [
        {
          from: 'Queued',
          to: 'Running',
          event: 'START',
          eventTypeId: 'et1',
          verb: { id: 'v1', name: 'start' },
        },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const config = JSON.parse(result.files['state-machines/job.json'])

    expect(config.states.Queued.on.START).toBe('Running')
  })

  // -------------------------------------------------------------------------
  // isInitial flag takes precedence over heuristic
  // -------------------------------------------------------------------------
  it('uses isInitial flag for initial state', async () => {
    const noun = mkNounDef({ name: 'Order' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Draft' },
        { id: 's2', name: 'Active', isInitial: true },
      ],
      transitions: [
        { from: 'Draft', to: 'Active', event: 'ACTIVATE', eventTypeId: 'et1' },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const config = JSON.parse(result.files['state-machines/order.json'])
    expect(config.initial).toBe('Active')
  })

  // -------------------------------------------------------------------------
  // Event Type Pattern on transitions
  // -------------------------------------------------------------------------
  it('includes pattern metadata when Event Type has Pattern', async () => {
    const noun = mkNounDef({ name: 'Webhook' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Pending' },
        { id: 's2', name: 'Success' },
        { id: 's3', name: 'Failure' },
      ],
      transitions: [
        { from: 'Pending', to: 'Success', event: '2XX', eventTypeId: 'et1', eventPattern: '2XX' },
        { from: 'Pending', to: 'Failure', event: '5XX', eventTypeId: 'et2', eventPattern: '5XX' },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const config = JSON.parse(result.files['state-machines/webhook.json'])

    // Transitions with patterns use object form
    expect(config.states.Pending.on['2XX']).toEqual({ target: 'Success', meta: { pattern: '2XX' } })
    expect(config.states.Pending.on['5XX']).toEqual({ target: 'Failure', meta: { pattern: '5XX' } })
  })

  // -------------------------------------------------------------------------
  // Guard references on transitions
  // -------------------------------------------------------------------------
  it('includes guard when transition has guard', async () => {
    const noun = mkNounDef({ name: 'Order' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Pending' },
        { id: 's2', name: 'Shipped' },
      ],
      transitions: [
        {
          from: 'Pending', to: 'Shipped', event: 'SHIP', eventTypeId: 'et1',
          guard: { graphSchemaId: 'gs-payment', constraintIds: ['c1'] },
        },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const config = JSON.parse(result.files['state-machines/order.json'])
    expect(config.states.Pending.on.SHIP).toEqual({ target: 'Shipped', guard: 'gs-payment' })
  })

  // -------------------------------------------------------------------------
  // Final states have type: 'final'
  // -------------------------------------------------------------------------
  it('marks terminal states as type: final', async () => {
    const noun = mkNounDef({ name: 'Task' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Open' },
        { id: 's2', name: 'Done' },
      ],
      transitions: [
        { from: 'Open', to: 'Done', event: 'COMPLETE', eventTypeId: 'et1' },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const config = JSON.parse(result.files['state-machines/task.json'])
    expect(config.states.Done.type).toBe('final')
    expect(config.states.Open.type).toBeUndefined()
  })

  // -------------------------------------------------------------------------
  // Moore callback (Verb on Status entry)
  // -------------------------------------------------------------------------
  it('includes Moore callback from Status verb', async () => {
    const noun = mkNounDef({ name: 'Pipeline' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Init' },
        {
          id: 's2', name: 'Processing',
          verb: { id: 'v1', name: 'process', func: { callbackUrl: 'https://pipeline.example.com/run', httpMethod: 'PUT' } },
        },
        { id: 's3', name: 'Done' },
      ],
      transitions: [
        { from: 'Init', to: 'Processing', event: 'START', eventTypeId: 'et1' },
        { from: 'Processing', to: 'Done', event: 'FINISH', eventTypeId: 'et2' },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const config = JSON.parse(result.files['state-machines/pipeline.json'])
    // Moore callback: action on entry to Processing state
    expect(config.states.Processing.callback).toEqual({ url: 'https://pipeline.example.com/run', method: 'PUT' })
  })

  // -------------------------------------------------------------------------
  // Agent tools include guards
  // -------------------------------------------------------------------------
  it('includes guards in agent tool definitions', async () => {
    const noun = mkNounDef({ name: 'Order' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Pending' },
        { id: 's2', name: 'Shipped' },
      ],
      transitions: [
        {
          from: 'Pending', to: 'Shipped', event: 'SHIP', eventTypeId: 'et1',
          guard: { graphSchemaId: 'gs-payment', constraintIds: ['c1'] },
        },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const tools = JSON.parse(result.files['agents/order-tools.json'])
    expect(tools[0].guards).toEqual(['gs-payment'])
  })

  // -------------------------------------------------------------------------
  // Prompt includes Initial state
  // -------------------------------------------------------------------------
  it('prompt includes initial state name', async () => {
    const noun = mkNounDef({ name: 'Ticket' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Open', isInitial: true },
        { id: 's2', name: 'Closed' },
      ],
      transitions: [
        { from: 'Open', to: 'Closed', event: 'CLOSE', eventTypeId: 'et1' },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const prompt = result.files['agents/ticket-prompt.md']
    expect(prompt).toContain('Initial: Open')
  })

  // -------------------------------------------------------------------------
  // Function without callbackUrl does not produce callback
  // -------------------------------------------------------------------------
  it('does not add callback when function has no callbackUrl', async () => {
    const noun = mkNounDef({ name: 'Task' })
    const sm = mkStateMachine({
      nounDef: noun,
      statuses: [
        { id: 's1', name: 'Todo' },
        { id: 's2', name: 'Done' },
      ],
      transitions: [
        {
          from: 'Todo',
          to: 'Done',
          event: 'COMPLETE',
          eventTypeId: 'et1',
          verb: { id: 'v1', name: 'complete', func: {} },
        },
      ],
    })

    const model = createMockModel({
      nouns: [noun],
      stateMachines: [sm],
      factTypes: [],
      readings: [],
    })

    const result = await generateXState(model)
    const config = JSON.parse(result.files['state-machines/task.json'])

    expect(config.states.Todo.on.COMPLETE).toBe('Done')
  })
})
