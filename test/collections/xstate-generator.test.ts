import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'
import { seedStateMachine, seedSupportDomain } from '../helpers/seed'

let payload: any
let output: any

describe('XState Generator', () => {
  beforeAll(async () => {
    payload = await initPayload()
    await payload.db.connection.dropDatabase()

    // Seed domain readings AND state machine
    await seedSupportDomain(payload)
    await seedStateMachine(payload)

    const generator = await payload.create({
      collection: 'generators',
      data: {
        title: 'Support State Machines',
        version: '1.0.0',
        databaseEngine: 'Payload',
        outputFormat: 'xstate',
      },
    })

    output = generator.output
  }, 120_000)

  it('should generate state machine files in output.files', () => {
    expect(output.files).toBeDefined()
    const smFiles = Object.keys(output.files).filter(f => f.startsWith('state-machines/'))
    expect(smFiles.length).toBeGreaterThan(0)
  })

  it('should generate valid XState config with id and initial state', () => {
    const smFile = Object.entries(output.files).find(([k]) => k.startsWith('state-machines/'))?.[1] as string
    expect(smFile).toBeDefined()
    const config = JSON.parse(smFile)
    expect(config.id).toBeDefined()
    expect(config.initial).toBe('Received')
  })

  it('should include all states', () => {
    const smFile = Object.entries(output.files).find(([k]) => k.startsWith('state-machines/'))?.[1] as string
    const config = JSON.parse(smFile)
    expect(config.states.Received).toBeDefined()
    expect(config.states.Triaging).toBeDefined()
    expect(config.states.Investigating).toBeDefined()
    expect(config.states.WaitingOnCustomer).toBeDefined()
    expect(config.states.Resolved).toBeDefined()
  })

  it('should include transitions as events on states', () => {
    const smFile = Object.entries(output.files).find(([k]) => k.startsWith('state-machines/'))?.[1] as string
    const config = JSON.parse(smFile)
    expect(config.states.Received.on.triage).toBeDefined()
    expect(config.states.Triaging.on.investigate).toBeDefined()
    expect(config.states.Triaging.on.resolve).toBeDefined()
    expect(config.states.Investigating.on.requestInfo).toBeDefined()
    expect(config.states.Investigating.on.resolve).toBeDefined()
    expect(config.states.WaitingOnCustomer.on.customerRespond).toBeDefined()
  })

  it('should set correct transition targets', () => {
    const smFile = Object.entries(output.files).find(([k]) => k.startsWith('state-machines/'))?.[1] as string
    const config = JSON.parse(smFile)
    expect(config.states.Received.on.triage.target).toBe('Triaging')
    expect(config.states.WaitingOnCustomer.on.customerRespond.target).toBe('Investigating')
  })

  it('should generate agent tool schemas', () => {
    const toolsFile = Object.entries(output.files).find(([k]) => k.startsWith('agents/') && k.endsWith('-tools.json'))?.[1] as string
    expect(toolsFile).toBeDefined()
    const tools = JSON.parse(toolsFile)
    expect(Array.isArray(tools)).toBe(true)
    expect(tools.length).toBeGreaterThan(0)
  })

  it('should create a tool for each unique event type', () => {
    const toolsFile = Object.entries(output.files).find(([k]) => k.startsWith('agents/') && k.endsWith('-tools.json'))?.[1] as string
    const tools = JSON.parse(toolsFile)
    const toolNames = tools.map((t: any) => t.name)
    expect(toolNames).toContain('triage')
    expect(toolNames).toContain('investigate')
    expect(toolNames).toContain('requestInfo')
    expect(toolNames).toContain('customerRespond')
    expect(toolNames).toContain('resolve')
  })

  it('should include source and target states in tool descriptions', () => {
    const toolsFile = Object.entries(output.files).find(([k]) => k.startsWith('agents/') && k.endsWith('-tools.json'))?.[1] as string
    const tools = JSON.parse(toolsFile)
    const triageTool = tools.find((t: any) => t.name === 'triage')
    expect(triageTool.description).toContain('Received')
    expect(triageTool.description).toContain('Triaging')
  })

  it('should generate a system prompt file', () => {
    const promptFile = Object.entries(output.files).find(([k]) => k.startsWith('agents/') && k.endsWith('-prompt.md'))?.[1] as string
    expect(promptFile).toBeDefined()
    expect(typeof promptFile).toBe('string')
  })

  it('should include domain model from readings in prompt', () => {
    const promptFile = Object.entries(output.files).find(([k]) => k.startsWith('agents/') && k.endsWith('-prompt.md'))?.[1] as string
    expect(promptFile).toContain('SupportRequest')
    expect(promptFile).toContain('Customer')
  })

  it('should include state machine context in prompt', () => {
    const promptFile = Object.entries(output.files).find(([k]) => k.startsWith('agents/') && k.endsWith('-prompt.md'))?.[1] as string
    expect(promptFile).toContain('Received')
    expect(promptFile).toContain('triage')
  })

  it('should only include relevant readings in prompt, not all readings', () => {
    const promptFile = Object.entries(output.files).find(([k]) => k.startsWith('agents/') && k.endsWith('-prompt.md'))?.[1] as string
    // The prompt should include SupportRequest-related readings
    expect(promptFile).toContain('SupportRequest has Subject')
    expect(promptFile).toContain('Customer submits SupportRequest')
    // Count the reading lines — should be far fewer than total readings in DB
    const readingLines = promptFile.split('\n').filter((l: string) => l.startsWith('- '))
    // seedSupportDomain creates ~13 readings. Prompt should have roughly that many, not all.
    expect(readingLines.length).toBeLessThan(30)
  })

  it('should not include meta.callback on transitions without verbs', () => {
    const smFile = Object.entries(output.files).find(([k]) => k.startsWith('state-machines/'))?.[1] as string
    const config = JSON.parse(smFile)
    // The seedStateMachine helper creates transitions without verbs
    expect(config.states.Received.on.triage.meta).toBeUndefined()
  })
}, 120_000)

describe('XState Generator — meta.callback from verb/function chain', () => {
  let callbackPayload: any
  let callbackOutput: any

  beforeAll(async () => {
    callbackPayload = await initPayload()
    await callbackPayload.db.connection.dropDatabase()

    // Create a minimal entity noun for the state machine
    const ticket = await callbackPayload.create({
      collection: 'nouns',
      data: { name: 'Ticket', plural: 'tickets', objectType: 'entity' },
    })

    // Create a function with callbackUrl
    const notifyFunc = await callbackPayload.create({
      collection: 'functions',
      data: {
        name: 'NotifyWebhook',
        functionType: 'httpCallback',
        callbackUrl: 'https://hooks.example.com/notify',
        httpMethod: 'POST',
      },
    })

    // Create a function without callbackUrl (query type)
    const queryFunc = await callbackPayload.create({
      collection: 'functions',
      data: {
        name: 'LookupData',
        functionType: 'query',
        queryText: 'SELECT * FROM tickets',
      },
    })

    // Create verbs linked to functions
    const notifyVerb = await callbackPayload.create({
      collection: 'verbs',
      data: { name: 'notify', function: notifyFunc.id },
    })

    const lookupVerb = await callbackPayload.create({
      collection: 'verbs',
      data: { name: 'lookup', function: queryFunc.id },
    })

    // Create state machine definition
    const definition = await callbackPayload.create({
      collection: 'state-machine-definitions',
      data: { noun: { relationTo: 'nouns', value: ticket.id } },
    })

    // Create statuses
    const open = await callbackPayload.create({
      collection: 'statuses',
      data: { name: 'Open', stateMachineDefinition: definition.id },
    })
    const notified = await callbackPayload.create({
      collection: 'statuses',
      data: { name: 'Notified', stateMachineDefinition: definition.id },
    })
    const closed = await callbackPayload.create({
      collection: 'statuses',
      data: { name: 'Closed', stateMachineDefinition: definition.id },
    })

    // Create event types
    const sendNotification = await callbackPayload.create({
      collection: 'event-types',
      data: { name: 'sendNotification' },
    })
    const close = await callbackPayload.create({
      collection: 'event-types',
      data: { name: 'close' },
    })
    const check = await callbackPayload.create({
      collection: 'event-types',
      data: { name: 'check' },
    })

    // Transition WITH verb that has httpCallback function
    await callbackPayload.create({
      collection: 'transitions',
      data: {
        from: open.id,
        to: notified.id,
        eventType: sendNotification.id,
        verb: notifyVerb.id,
      },
    })

    // Transition WITHOUT a verb
    await callbackPayload.create({
      collection: 'transitions',
      data: {
        from: notified.id,
        to: closed.id,
        eventType: close.id,
      },
    })

    // Transition WITH verb that has a query function (no callbackUrl)
    await callbackPayload.create({
      collection: 'transitions',
      data: {
        from: open.id,
        to: open.id,
        eventType: check.id,
        verb: lookupVerb.id,
      },
    })

    // Seed a minimal reading so the noun is findable
    const gs = await callbackPayload.create({
      collection: 'graph-schemas',
      data: { name: 'TicketHasSubject' },
    })
    await callbackPayload.create({
      collection: 'readings',
      data: { text: 'Ticket has Subject', graphSchema: gs.id },
    })

    // Generate XState output
    const generator = await callbackPayload.create({
      collection: 'generators',
      data: {
        title: 'Ticket State Machines',
        version: '1.0.0',
        databaseEngine: 'Payload',
        outputFormat: 'xstate',
      },
    })

    callbackOutput = generator.output
  }, 120_000)

  it('should embed meta.callback on transitions with verb → httpCallback function', () => {
    const smFile = Object.entries(callbackOutput.files).find(([k]) => k.startsWith('state-machines/'))?.[1] as string
    expect(smFile).toBeDefined()
    const config = JSON.parse(smFile)
    const sendNotification = config.states.Open.on.sendNotification
    expect(sendNotification).toBeDefined()
    expect(sendNotification.target).toBe('Notified')
    expect(sendNotification.meta).toBeDefined()
    expect(sendNotification.meta.callback).toEqual({
      url: 'https://hooks.example.com/notify',
      method: 'POST',
    })
  })

  it('should not embed meta.callback on transitions without verbs', () => {
    const smFile = Object.entries(callbackOutput.files).find(([k]) => k.startsWith('state-machines/'))?.[1] as string
    const config = JSON.parse(smFile)
    const close = config.states.Notified.on.close
    expect(close).toBeDefined()
    expect(close.target).toBe('Closed')
    expect(close.meta).toBeUndefined()
  })

  it('should not embed meta.callback on transitions with verb → non-httpCallback function', () => {
    const smFile = Object.entries(callbackOutput.files).find(([k]) => k.startsWith('state-machines/'))?.[1] as string
    const config = JSON.parse(smFile)
    const check = config.states.Open.on.check
    expect(check).toBeDefined()
    expect(check.target).toBe('Open')
    expect(check.meta).toBeUndefined()
  })
}, 120_000)
