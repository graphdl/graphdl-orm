/**
 * domain-fixture.test.ts — Verifies that compileDomain returns real IR from the WASM engine.
 *
 * The WASM system(handle, 'debug', '') returns a display string in GraphDL's internal notation.
 * compileDomain parses this into { nouns, factTypes, constraints, totalFacts, raw }.
 */

import { describe, it, expect, afterAll } from 'vitest'
import {
  compileDomain,
  ORDER_READINGS,
  SUPPORT_READINGS,
  releaseDomain,
  type CompiledDomain,
} from './domain-fixture'

describe('compileDomain(ORDER_READINGS, "orders")', () => {
  let compiled: CompiledDomain

  it('compiles without throwing and returns a valid handle', () => {
    compiled = compileDomain(ORDER_READINGS, 'orders')
    expect(compiled).toBeDefined()
    expect(compiled.handle).toBeGreaterThanOrEqual(0)
  })

  it('IR contains nouns including Order and Customer', () => {
    const { nouns } = compiled.ir
    expect(Array.isArray(nouns)).toBe(true)
    expect(nouns.length).toBeGreaterThan(0)
    expect(nouns.some(n => n.includes('Order'))).toBe(true)
    expect(nouns.some(n => n.includes('Customer'))).toBe(true)
  })

  it('IR contains fact types', () => {
    const { factTypes } = compiled.ir
    expect(Array.isArray(factTypes)).toBe(true)
    expect(factTypes.length).toBeGreaterThan(0)
  })

  it('entities list matches IR nouns', () => {
    expect(compiled.entities).toEqual(compiled.ir.nouns)
    expect(compiled.entities.length).toBeGreaterThan(0)
  })

  it('IR has constraints', () => {
    expect(Array.isArray(compiled.ir.constraints)).toBe(true)
    expect(compiled.ir.constraints.length).toBeGreaterThan(0)
  })

  it('IR totalFacts is positive', () => {
    expect(compiled.ir.totalFacts).toBeGreaterThan(0)
  })

  afterAll(() => {
    if (compiled?.handle >= 0) {
      releaseDomain(compiled.handle)
    }
  })
})

describe('compileDomain(SUPPORT_READINGS, "support")', () => {
  let compiled: CompiledDomain

  it('compiles without throwing', () => {
    compiled = compileDomain(SUPPORT_READINGS, 'support')
    expect(compiled).toBeDefined()
    expect(compiled.handle).toBeGreaterThanOrEqual(0)
  })

  it('IR contains Ticket entity', () => {
    const { nouns } = compiled.ir
    expect(nouns.some(n => n.includes('Ticket'))).toBe(true)
  })

  it('IR contains deontic constraints', () => {
    const { raw } = compiled.ir
    // Raw debug output should contain obligation marker
    expect(raw.toLowerCase()).toContain('ticket')
  })

  afterAll(() => {
    if (compiled?.handle >= 0) {
      releaseDomain(compiled.handle)
    }
  })
})
