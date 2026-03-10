import { describe, it, expect } from 'vitest'
import { BOOTSTRAP_DML } from './bootstrap'
import { ALL_DDL } from './index'

describe('bootstrap DML', () => {
  it('contains INSERT OR IGNORE statements only', () => {
    for (const stmt of BOOTSTRAP_DML) {
      expect(stmt.trim()).toMatch(/^INSERT OR IGNORE INTO/)
    }
  })

  it('seeds the graphdl-core domain + at least 20 metamodel nouns', () => {
    const domainInserts = BOOTSTRAP_DML.filter(s => s.includes('INTO domains'))
    const nounInserts = BOOTSTRAP_DML.filter(s => s.includes('INTO nouns'))

    expect(domainInserts.length).toBe(1)
    expect(nounInserts.length).toBeGreaterThanOrEqual(20)
  })

  it('references valid supertype IDs for subtypes', () => {
    // All super_type_id references should point to nouns that are also bootstrapped
    const nounIds = new Set<string>()
    for (const stmt of BOOTSTRAP_DML) {
      const match = stmt.match(/INSERT OR IGNORE INTO nouns.*VALUES \('([^']+)'/)
      if (match) nounIds.add(match[1])
    }

    for (const stmt of BOOTSTRAP_DML) {
      if (!stmt.includes('INTO nouns')) continue
      const superMatch = stmt.match(/super_type_id.*?'(meta-[^']+)'/)
      if (superMatch) {
        expect(nounIds).toContain(superMatch[1])
      }
    }
  })

  it('uses deterministic IDs prefixed with meta-', () => {
    for (const stmt of BOOTSTRAP_DML) {
      if (!stmt.includes('INTO nouns')) continue
      const idMatch = stmt.match(/VALUES \('(meta-[^']+)'/)
      expect(idMatch).toBeTruthy()
    }
  })
})
