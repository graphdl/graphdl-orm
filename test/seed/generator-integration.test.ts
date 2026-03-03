import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'
import { parseDomainMarkdown } from '../../src/seed/parser'
import { seedDomain } from '../../src/seed/handler'
import fs from 'fs'
import path from 'path'

let payload: any

const DOMAINS_DIR = path.resolve(__dirname, '../../../auto.dev-graphdl/domains')

describe('Generator with real seeded domains', () => {
  beforeAll(async () => {
    payload = await initPayload()
    await payload.db.connection.dropDatabase()

    // Seed all domains
    const files = fs.readdirSync(DOMAINS_DIR).filter((f: string) => f.endsWith('.md'))
    for (const f of files) {
      const md = fs.readFileSync(path.join(DOMAINS_DIR, f), 'utf-8')
      const parsed = parseDomainMarkdown(md)
      const domain = f.replace('.md', '')
      const result = await seedDomain(payload, parsed, domain)
      console.log(`  ${domain}: ${result.nouns}n ${result.readings}r ${result.errors.length}err`)
    }
  }, 300_000)

  it('should generate OpenAPI for all domains', async () => {
    const gen = await payload.create({
      collection: 'generators',
      data: { title: 'Full API', version: '1.0.0', databaseEngine: 'Payload' },
    })
    const schemas = gen.output?.components?.schemas || {}
    console.log('Total schemas:', Object.keys(schemas).length)
    expect(Object.keys(schemas).length).toBeGreaterThan(0)
  }, 120_000)
})
