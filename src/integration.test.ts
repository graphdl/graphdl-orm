import { describe, it, expect } from 'vitest'
import { parseFORML2 } from './api/parse'
import { runCsdpPipeline } from './csdp/pipeline'
import { generateSQLiteFromRmap } from './generate/sqlite'
import { generateOpenAPIFromRmap } from './generate/openapi'

describe('end-to-end pipeline', () => {
  const FORML2_TEXT = `
Customer(.Email) is an entity type.
Order(.OrderNumber) is an entity type.
Name is a value type.
Status is a value type.
  The possible values of Status are 'pending', 'shipped', 'delivered'.

Customer has Name.
  Each Customer has exactly one Name.
Customer places Order.
  Each Order is placed by at most one Customer.
Order has Status.
  Each Order has exactly one Status.
`

  it('parses FORML2 text into claims', () => {
    const claims = parseFORML2(FORML2_TEXT, [])
    // 4 declared nouns: Customer, Order, Name, Status
    // Plus implicit value types from ref schemes: Email, OrderNumber
    expect(claims.nouns.length).toBeGreaterThanOrEqual(4)
    // Readings: Customer has Email (implicit), Order has OrderNumber (implicit),
    //           Customer has Name, Customer places Order, Order has Status
    expect(claims.readings.length).toBeGreaterThanOrEqual(3)
    // Constraints: MC on Email (implicit), MC on OrderNumber (implicit),
    //              UC on Customer has Name, UC on Customer places Order,
    //              UC on Order has Status
    expect(claims.constraints.length).toBeGreaterThanOrEqual(3)
  })

  it('runs CSDP pipeline and produces valid batch', () => {
    const claims = parseFORML2(FORML2_TEXT, [])
    const result = runCsdpPipeline(claims, 'ecommerce')
    expect(result.valid).toBe(true)
    expect(result.batch).toBeDefined()
    expect(result.batch!.entities.length).toBeGreaterThan(0)

    // Batch should contain Nouns, Readings, Constraints
    const types = new Set(result.batch!.entities.map(e => e.type))
    expect(types.has('Noun')).toBe(true)
    expect(types.has('Reading')).toBe(true)
  })

  it('produces RMAP tables from CSDP pipeline', () => {
    const claims = parseFORML2(FORML2_TEXT, [])
    const result = runCsdpPipeline(claims, 'ecommerce')
    expect(result.valid).toBe(true)
    expect(result.tables).toBeDefined()
    expect(result.tables!.length).toBeGreaterThan(0)

    // Should have a customer table and an order table
    const tableNames = result.tables!.map(t => t.name)
    expect(tableNames.some(n => n.includes('customer'))).toBe(true)
    expect(tableNames.some(n => n.includes('order'))).toBe(true)
  })

  it('generates SQLite DDL from RMAP output', () => {
    const claims = parseFORML2(FORML2_TEXT, [])
    const result = runCsdpPipeline(claims, 'ecommerce')
    const ddl = generateSQLiteFromRmap(result.tables!)
    expect(ddl).toContain('CREATE TABLE')
    expect(ddl).toContain('NOT NULL') // mandatory constraints
    expect(ddl).toContain('PRIMARY KEY')
  })

  it('generates OpenAPI from RMAP output', () => {
    const claims = parseFORML2(FORML2_TEXT, [])
    const result = runCsdpPipeline(claims, 'ecommerce')
    const api = generateOpenAPIFromRmap(result.tables!, 'ecommerce')
    expect(api).toHaveProperty('openapi')
    expect(api).toHaveProperty('components')
  })

  it('CSDP pipeline runs without crashing on edge cases', () => {
    const badClaims = parseFORML2(`
A(.id) is an entity type.
B(.id) is an entity type.
C(.id) is an entity type.
A has B for C.
  Each A has at most one B.
`, [])
    // At minimum, verify the pipeline runs without crashing and
    // returns a structured result with the expected shape
    const result = runCsdpPipeline(badClaims, 'test')
    expect(result).toHaveProperty('valid')
    expect(result).toHaveProperty('violations')
  })

  it('handles subtypes correctly', () => {
    const claims = parseFORML2(`
Person(.Name) is an entity type.
Male is a subtype of Person.
Female is a subtype of Person.
No Person is both a Male and a Female.
`, [])
    const result = runCsdpPipeline(claims, 'test')
    // The CSDP validator may flag missing totality constraint, but the
    // pipeline should still produce a result without crashing.
    // The exclusion constraint should satisfy the subtype constraint check.
    expect(result).toHaveProperty('valid')
    expect(result).toHaveProperty('violations')
    // Regardless of valid/invalid, verify Person noun is present in claims
    expect(claims.nouns.some(n => n.name === 'Person')).toBe(true)
    expect(claims.subtypes!.length).toBe(2)
    expect(claims.subtypes!.some(s => s.child === 'Male' && s.parent === 'Person')).toBe(true)
    expect(claims.subtypes!.some(s => s.child === 'Female' && s.parent === 'Person')).toBe(true)
  })

  it('handles state machine transitions (table format)', () => {
    const claims = parseFORML2(`
# Order Lifecycle

Order(.OrderNumber) is an entity type.
Status is a value type.
  The possible values of Status are 'pending', 'shipped', 'delivered'.
Order has Status.
  Each Order has exactly one Status.

## Transitions
| From | To | Event |
| --- | --- | --- |
| pending | shipped | ship |
| shipped | delivered | deliver |
`, [])
    expect(claims.transitions!.length).toBeGreaterThanOrEqual(2)
    expect(claims.transitions!.some(t => t.from === 'pending' && t.to === 'shipped')).toBe(true)
    expect(claims.transitions!.some(t => t.from === 'shipped' && t.to === 'delivered')).toBe(true)
  })

  it('round-trips the full pipeline: parse → CSDP → RMAP → SQLite + OpenAPI', () => {
    // Single end-to-end pass verifying every stage produces output
    const claims = parseFORML2(FORML2_TEXT, [])
    expect(claims.nouns.length).toBeGreaterThan(0)

    const result = runCsdpPipeline(claims, 'ecommerce')
    expect(result.valid).toBe(true)
    expect(result.batch!.entities.length).toBeGreaterThan(0)
    expect(result.tables!.length).toBeGreaterThan(0)

    const ddl = generateSQLiteFromRmap(result.tables!)
    expect(ddl.length).toBeGreaterThan(0)

    const api = generateOpenAPIFromRmap(result.tables!, 'ecommerce') as any
    expect(Object.keys(api.components.schemas).length).toBeGreaterThan(0)
  })
})
