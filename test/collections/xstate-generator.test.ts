import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'
import { seedStateMachine } from '../helpers/seed'

let payload: any
let output: any

describe('XState Generator', () => {
  beforeAll(async () => {
    payload = await initPayload()
    await payload.db.connection.dropDatabase()

    await seedStateMachine(payload)

    const generator = await payload.create({
      collection: 'generators',
      data: {
        title: 'Support State Machines',
        version: '1.0.0',
        databaseEngine: 'Payload',
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
}, 120_000)
