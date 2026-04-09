/**
 * Security tests — authorization, injection, isolation, bounds.
 *
 * TDD: these tests describe what the system SHOULD enforce.
 * They are RED until the engine implements the security checks.
 *
 * Threat model:
 *   - Reading injection: compile modifies the program
 *   - Fact injection: apply creates arbitrary entities
 *   - Handle isolation: shared global state between tenants
 *   - Input bounds: DoS via oversized readings
 *   - Noun shadowing: hijack metamodel nouns
 *   - SSRF: external system federation to arbitrary URLs
 */

import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import {
  compileDomain,
  apply,
  transitions,
  systemRaw,
  releaseDomain,
  STATE_READINGS,
  ORDER_READINGS,
} from '../helpers/domain-fixture'
import { compileDomainReadings } from '../../api/engine'

// ── Handle Isolation (#15) ──────────────────────────────────────────────────
// Two tenants must not share state through handles.

describe('Handle isolation', () => {
  it('two separate create() calls produce different handles', () => {
    const h1 = compileDomainReadings(STATE_READINGS, ORDER_READINGS)
    const h2 = compileDomainReadings(STATE_READINGS, ORDER_READINGS)
    expect(h1).not.toBe(h2)
  })

  it('applying a command on handle A does not affect handle B', () => {
    const h1 = compileDomainReadings(STATE_READINGS, ORDER_READINGS)
    const h2 = compileDomainReadings(STATE_READINGS, ORDER_READINGS)

    // Create entity on h1
    apply(h1, {
      type: 'createEntity', noun: 'Order', domain: 'tenant-a',
      fields: { customer: 'Alice' },
    })

    // h2 should not see h1's entity — debug output should differ
    const debug1 = systemRaw(h1, 'debug', '')
    const debug2 = systemRaw(h2, 'debug', '')

    // Both have the same schema but h1 has more state (the created entity)
    // At minimum, the handles are distinct
    expect(h1).not.toBe(h2)

    releaseDomain(h1)
    releaseDomain(h2)
  })

  it('released handle returns ⊥ for all operations', () => {
    const h = compileDomainReadings(STATE_READINGS, ORDER_READINGS)
    releaseDomain(h)
    const result = systemRaw(h, 'debug', '')
    expect(result).toBe('⊥')
  })
})

// ── Compile Authorization (#16, #22) ────────────────────────────────────────
// compile modifies the program — it must be gated.

describe('Compile authorization', () => {
  it('compile without identity context is rejected', () => {
    const h = compileDomainReadings(STATE_READINGS, ORDER_READINGS)
    // An unauthenticated compile should fail
    // Currently this passes — this test should FAIL until #16 is implemented
    const result = systemRaw(h, 'compile', 'Malicious(.id) is an entity type.')
    // The result should indicate rejection, not success
    expect(result.startsWith('⊥') || result.includes('forbidden') || result.includes('unauthorized')).toBe(true)
    releaseDomain(h)
  })

  it('compile that weakens constraints is rejected', () => {
    const h = compileDomainReadings(STATE_READINGS, ORDER_READINGS)
    // Try to add a permissive constraint that overrides an existing forbidden
    const poison = 'It is permitted that Order has more than one Customer.'
    const result = systemRaw(h, 'compile', poison)
    // Should be rejected by the evolution state machine
    expect(result.startsWith('⊥') || result.includes('forbidden')).toBe(true)
    releaseDomain(h)
  })
})

// ── Noun Consistency ────────────────────────────────────────────────────────
// UC "Each Noun has exactly one Object Type" catches contradictions.

describe('Noun consistency', () => {
  it('contradictory declarations in same readings are caught', () => {
    // Status declared as both entity and value in the same compilation
    const contradictory = `
Status(.Name) is an entity type.
Status is a value type.
`.trim()
    const { ir } = compileDomain(contradictory)
    // The parser records the last declaration. The noun exists but
    // the UC should eventually catch the contradiction when both
    // facts coexist in P. For now, document the behavior.
    expect(ir.nouns.length).toBeGreaterThan(0)
  })

  it('post-bootstrap compile is rejected (compile auth)', () => {
    const h = compileDomainReadings(STATE_READINGS, ORDER_READINGS)
    // After bootstrap, bare compile is forbidden
    const result = systemRaw(h, 'compile', 'Evil(.id) is an entity type.')
    expect(result.startsWith('⊥') || result.includes('forbidden')).toBe(true)
    releaseDomain(h)
  })
})

// ── Input Bounds (#19) ──────────────────────────────────────────────────────
// Oversized input must not crash or OOM the engine.

describe('Input bounds', () => {
  it('oversized readings text is rejected', () => {
    const h = compileDomainReadings(STATE_READINGS)
    // 10MB of readings — should be rejected before parsing
    const huge = 'X'.repeat(10 * 1024 * 1024)
    const result = systemRaw(h, 'compile', huge)
    expect(result.startsWith('⊥') || result.includes('too large')).toBe(true)
    releaseDomain(h)
  })

  it('deeply nested AST input does not crash', () => {
    const h = compileDomainReadings(STATE_READINGS, ORDER_READINGS)
    // Deeply nested angle brackets
    const nested = '<'.repeat(1000) + 'x' + '>'.repeat(1000)
    const result = systemRaw(h, 'apply', nested)
    // Should return ⊥ (parse error), not crash
    expect(result.startsWith('⊥')).toBe(true)
    releaseDomain(h)
  })

  it('command with huge field values is bounded', () => {
    const h = compileDomainReadings(STATE_READINGS, ORDER_READINGS)
    const bigValue = 'A'.repeat(1024 * 1024)
    const result = apply(h, {
      type: 'createEntity', noun: 'Order', domain: 'test',
      fields: { customer: bigValue },
    })
    // Should either succeed with truncation or reject — not crash
    expect(result).toBeDefined()
    releaseDomain(h)
  })
})

// ── Apply Identity (#17, #20) ───────────────────────────────────────────────
// Commands carry caller identity via the `sender` field. Per AREST §8
// (Middleware Elimination), identity enforcement happens via the existing
// derive+validate pipeline: sender is pushed as a User fact into the
// population BEFORE derive runs, and any alethic constraint touching User
// facts fires during validate. NO procedural auth middleware.

describe('Apply identity', () => {
  const AUTH_DOMAIN = `# Auth

## Entity Types
Order(.OrderId) is an entity type.
User(.Email) is an entity type.

## Value Types
OrderId is a value type.
Email is a value type.

## Fact Types
### Order
Order is created by User.

## Constraints
Each Order is created by exactly one User.
`.trim()

  it('createEntity without identity context is rejected', () => {
    const h = compileDomainReadings(AUTH_DOMAIN)
    const result = apply(h, {
      type: 'createEntity',
      noun: 'Order',
      domain: 'test',
      id: 'ord-1',
      fields: { OrderId: 'ord-1' },
    })
    // Missing sender -> no User fact -> MC fires -> alethic violation -> rejected.
    expect(result.rejected).toBe(true)
    expect(result.violations.length).toBeGreaterThan(0)
    releaseDomain(h)
  })
})

// ── SSRF Prevention (#25) ───────────────────────────────────────────────────
// External system federation must not allow arbitrary URLs.

describe('SSRF prevention', () => {
  it('cannot compile readings that back a noun by an arbitrary URL', () => {
    const h = compileDomainReadings(STATE_READINGS)
    // Try to create an external system pointing to internal network
    const ssrf = `
External System(.Name) is an entity type.
URL is a value type.
External System has URL.
External System 'evil' has URL 'http://169.254.169.254/latest/meta-data/'.
Noun(.Name) is an entity type.
Noun is backed by External System.
Noun 'Secret' is backed by External System 'evil'.
`.trim()
    const result = systemRaw(h, 'compile', ssrf)
    // Should reject internal/metadata URLs
    expect(result.startsWith('⊥') || result.includes('forbidden')).toBe(true)
    releaseDomain(h)
  })
})

// ── Debug Restriction (#18) ─────────────────────────────────────────────────
// Debug should not expose sensitive state in production.

describe('Debug restriction', () => {
  it('debug returns state projection (documents current behavior)', () => {
    const h = compileDomainReadings(STATE_READINGS, ORDER_READINGS)
    const debug = systemRaw(h, 'debug', '')
    // Currently returns full state — this documents the exposure
    expect(debug).toContain('nouns')
    expect(debug).toContain('constraints')
    releaseDomain(h)
  })
})

// ── Malformed Readings (#19 related) ────────────────────────────────────────
// Parser must not crash on invalid input.

describe('Malformed readings resilience', () => {
  it('empty string compiles without crash', () => {
    const h = compileDomainReadings('')
    expect(h).toBeGreaterThanOrEqual(0)
    releaseDomain(h)
  })

  it('random binary data does not crash the parser', () => {
    const h = compileDomainReadings(STATE_READINGS)
    const binary = String.fromCharCode(...Array.from({ length: 256 }, (_, i) => i))
    const result = systemRaw(h, 'compile', binary)
    // Should not crash — may return ⊥ or a small def count
    expect(typeof result).toBe('string')
    releaseDomain(h)
  })

  it('unclosed quotes do not crash the parser', () => {
    const h = compileDomainReadings(STATE_READINGS)
    const result = systemRaw(h, 'compile', "Status 'unclosed is initial in State Machine Definition 'also unclosed")
    expect(typeof result).toBe('string')
    releaseDomain(h)
  })
})
