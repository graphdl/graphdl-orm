/**
 * Anonymous Peers — Signatures, Authorization, Chain Integrity
 *
 * Tests Section 6.3 of the AREST paper:
 * - Identity is established via signatures (author stored in event data)
 * - Authorization is a deontic constraint checked before applying events
 * - The hash chain provides ordering and tamper-evident integrity
 */

import { describe, it, expect } from 'vitest'
import { CellChain } from '../../mcp/hash-chain'
import { type CellEvent } from '../../mcp/streaming'

// ── Authorization constraint ───────────────────────────────────────────────

type AuthConstraint = (event: any, author: string) => string | null

const schemaModificationRequiresAdmin: AuthConstraint = (event, author) => {
  if (event.type === 'compile_parse' && author !== 'admin') {
    return 'It is forbidden that a Domain Change is applied without Signal Source Human.'
  }
  return null
}

// ── Helpers ────────────────────────────────────────────────────────────────

function ev(
  seq: number,
  facts: Record<string, unknown>,
  ts?: number,
  entityId = 'e1',
): CellEvent {
  return {
    domain: 'test',
    noun: 'Entity',
    entityId,
    operation: 'update',
    facts,
    timestamp: ts ?? seq * 1000,
    sequence: seq,
  }
}

// ── Identity via signatures ────────────────────────────────────────────────

describe('Identity via signatures', () => {
  it('events carry author identity stored in event data as _author', () => {
    const chain = new CellChain()
    const event = ev(1, { value: 42, _author: 'peer-alice' })
    const chained = chain.append(event)

    expect(chained.facts._author).toBe('peer-alice')
  })

  it('events from different peers are distinguishable by _author', () => {
    const aliceChain = new CellChain()
    const bobChain = new CellChain()

    const aliceEvent = ev(1, { status: 'ready', _author: 'peer-alice' })
    const bobEvent = ev(1, { status: 'ready', _author: 'peer-bob' })

    const chainedAlice = aliceChain.append(aliceEvent)
    const chainedBob = bobChain.append(bobEvent)

    expect(chainedAlice.facts._author).toBe('peer-alice')
    expect(chainedBob.facts._author).toBe('peer-bob')
    expect(chainedAlice.facts._author).not.toBe(chainedBob.facts._author)
    // Different authors produce different chain tips even with same payload shape
    expect(chainedAlice.hash).not.toBe(chainedBob.hash)
  })
})

// ── Authorization as deontic constraint ───────────────────────────────────

describe('Authorization as deontic constraint', () => {
  it('schema modification by unauthorized peer is rejected with "forbidden" message', () => {
    const unauthorizedEvent = { type: 'compile_parse', payload: { schema: 'new schema' } }
    const author = 'peer-alice'

    const violation = schemaModificationRequiresAdmin(unauthorizedEvent, author)

    expect(violation).not.toBeNull()
    expect(violation).toContain('forbidden')
  })

  it('schema modification by authorized peer (admin) is accepted', () => {
    const schemaEvent = { type: 'compile_parse', payload: { schema: 'new schema' } }
    const author = 'admin'

    const violation = schemaModificationRequiresAdmin(schemaEvent, author)

    expect(violation).toBeNull()
  })

  it('normal operations do not require special authorization', () => {
    const normalEvents = [
      { type: 'create', payload: { name: 'Widget' } },
      { type: 'update', payload: { status: 'active' } },
      { type: 'delete', payload: { id: '123' } },
      { type: 'transition', payload: { state: 'shipped' } },
    ]

    for (const event of normalEvents) {
      const violation = schemaModificationRequiresAdmin(event, 'peer-alice')
      expect(violation).toBeNull()
    }
  })
})

// ── Chain provides ordering and integrity ──────────────────────────────────

describe('Chain provides ordering and integrity', () => {
  it('hash chain is tamper-evident — verify() detects mutation', () => {
    const chain = new CellChain()
    chain.append(ev(1, { status: 'placed' }))
    const e2 = chain.append(ev(2, { status: 'shipped' }))
    chain.append(ev(3, { status: 'delivered' }))

    expect(chain.verify()).toBe(true)

    // Tamper with a fact in the second event
    ;(e2 as any).facts = { status: 'cancelled' }

    expect(chain.verify()).toBe(false)
  })

  it('every event references the hash of its predecessor', () => {
    const chain = new CellChain()
    const e1 = chain.append(ev(1, { a: 1 }))
    const e2 = chain.append(ev(2, { b: 2 }))
    const e3 = chain.append(ev(3, { c: 3 }))

    expect(e1.prevHash).toBe('0') // genesis sentinel
    expect(e2.prevHash).toBe(e1.hash)
    expect(e3.prevHash).toBe(e2.hash)
  })

  it('peers compare chain tips — identical tips mean agreement', () => {
    const peerA = new CellChain()
    const peerB = new CellChain()

    // Both peers process the same ordered log
    const sharedLog = [
      ev(1, { status: 'placed' }, 1000),
      ev(2, { status: 'shipped' }, 2000),
      ev(3, { status: 'delivered' }, 3000),
    ]

    for (const event of sharedLog) {
      peerA.append(event)
      peerB.append(event)
    }

    // Identical tips prove agreement without sharing full history
    expect(peerA.getTip()).toBe(peerB.getTip())
    expect(peerA.verify()).toBe(true)
    expect(peerB.verify()).toBe(true)
  })
})
