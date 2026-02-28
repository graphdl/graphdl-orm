import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from './helpers/initPayload'

let payload: any

describe('Smoke test', () => {
  beforeAll(async () => {
    payload = await initPayload()
  })

  it('should initialize payload', () => {
    expect(payload).toBeDefined()
    expect(payload.collections).toBeDefined()
  })

  it('should create and find a noun', async () => {
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'TestNoun', objectType: 'entity' },
    })
    expect(noun.id).toBeDefined()
    expect(noun.name).toBe('TestNoun')

    const found = await payload.findByID({ collection: 'nouns', id: noun.id })
    expect(found.name).toBe('TestNoun')
  })
})
