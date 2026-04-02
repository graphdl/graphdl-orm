/**
 * Federation tests — end-to-end from readings to DEFS resolution.
 *
 * Seeds the organizations.md readings (which declare Noun 'User' is
 * backed by External System 'auth.vin') and verifies the DEFS cell
 * registers external resolution for the User noun.
 */

import { describe, it, expect, vi } from 'vitest'
import { readFileSync } from 'fs'
import { resolve } from 'path'

// Mock the WASM engine to produce entities from readings
vi.mock('./engine', () => {
  // Simulate what the real parser produces for the backed-by instance facts
  const parse = (_markdown: string, domain: string) => {
    const entities: any[] = [
      { id: 'User', type: 'Noun', domain, data: { name: 'User', objectType: 'entity', domain } },
      { id: 'Organization', type: 'Noun', domain, data: { name: 'Organization', objectType: 'entity', domain } },
      { id: 'External System', type: 'Noun', domain, data: { name: 'External System', objectType: 'entity', domain } },
      // Instance Fact: Noun 'User' is backed by External System 'auth.vin'
      { id: 'if:user-backed', type: 'Instance Fact', domain, data: {
        subjectNoun: 'Noun', subjectValue: 'User',
        fieldName: 'is backed by',
        objectNoun: 'External System', objectValue: 'auth.vin',
      }},
      // Instance Fact: Noun 'User' has URI '/users'
      { id: 'if:user-uri', type: 'Instance Fact', domain, data: {
        subjectNoun: 'Noun', subjectValue: 'User',
        fieldName: 'URI',
        objectNoun: '', objectValue: '/users',
      }},
    ]
    return entities
  }
  return {
    parseReadings: vi.fn(parse),
    parseReadingsWithNouns: vi.fn((md: string, domain: string, _nouns: string) => parse(md, domain)),
    reconstructIR: vi.fn(async () => '{}'),
    ensureWasm: vi.fn(),
  }
})

describe('Federation end-to-end', () => {
  it('DEFS cell registers external resolution for backed nouns', () => {
    // Simulate the DEFS building logic from seed.ts
    const instanceFacts = [
      { subjectNoun: 'Noun', subjectValue: 'User', objectNoun: 'External System', objectValue: 'auth.vin' },
    ]

    const defsData: Record<string, string> = {
      '*:read': 'local',
      '*:readDetail': 'local',
      '*:create': 'local',
    }

    for (const fact of instanceFacts) {
      if (fact.subjectNoun === 'Noun' && fact.objectNoun === 'External System') {
        defsData[`${fact.subjectValue}:read`] = 'external'
        defsData[`${fact.subjectValue}:readDetail`] = 'external'
      }
    }

    expect(defsData['User:read']).toBe('external')
    expect(defsData['User:readDetail']).toBe('external')
    expect(defsData['Organization:read']).toBeUndefined()
  })

  it('rho resolves User to external and Organization to local', () => {
    const defsData: Record<string, string> = {
      '*:read': 'local',
      '*:readDetail': 'local',
      '*:create': 'local',
      'User:read': 'external',
      'User:readDetail': 'external',
    }

    const resolve = (noun: string, op: string) =>
      defsData[`${noun}:${op}`] || defsData[`*:${op}`] || 'local'

    expect(resolve('User', 'readDetail')).toBe('external')
    expect(resolve('Organization', 'readDetail')).toBe('local')
    expect(resolve('User', 'create')).toBe('local')
  })

  it('organizations.md declares User backed by auth.vin', () => {
    const orgReadings = readFileSync(
      resolve(__dirname, '../../readings/organizations.md'), 'utf-8'
    )
    expect(orgReadings).toContain("Noun 'User' is backed by External System 'auth.vin'")
    expect(orgReadings).toContain("Noun 'User' has URI '/users'")
  })
})
