/**
 * Hash chain tests — peer-to-peer event ordering and convergence.
 */

import { describe, it, expect } from 'vitest'
import { CellChain, mergeChains, eventHash } from './hash-chain'
import { type CellEvent } from './streaming'

function ev(seq: number, facts: Record<string, unknown>, ts?: number): CellEvent {
  return { domain: 'test', noun: 'Entity', entityId: 'e1', operation: 'update', facts, timestamp: ts || seq * 1000, sequence: seq }
}

describe('CellChain', () => {
  it('appends events with hash links', () => {
    const chain = new CellChain()
    const e1 = chain.append(ev(1, { value: 10 }))
    const e2 = chain.append(ev(2, { value: 20 }))

    expect(e1.prevHash).toBe('0') // genesis
    expect(e2.prevHash).toBe(e1.hash)
    expect(chain.length()).toBe(2)
  })

  it('verifies chain integrity', () => {
    const chain = new CellChain()
    chain.append(ev(1, { a: 1 }))
    chain.append(ev(2, { b: 2 }))
    chain.append(ev(3, { c: 3 }))

    expect(chain.verify()).toBe(true)
  })

  it('detects tampered chain', () => {
    const chain = new CellChain()
    chain.append(ev(1, { a: 1 }))
    const e2 = chain.append(ev(2, { b: 2 }))

    // Tamper with the event
    ;(e2 as any).facts = { b: 999 }

    expect(chain.verify()).toBe(false)
  })

  it('deterministic: same events produce same hashes', () => {
    const chainA = new CellChain()
    const chainB = new CellChain()

    const events = [ev(1, { x: 1 }, 1000), ev(2, { y: 2 }, 2000)]

    for (const e of events) {
      chainA.append(e)
      chainB.append(e)
    }

    expect(chainA.getTip()).toBe(chainB.getTip())
    expect(chainA.getEvents().map(e => e.hash)).toEqual(chainB.getEvents().map(e => e.hash))
  })
})

describe('Fork detection', () => {
  it('no fork when chains are identical', () => {
    const a = new CellChain()
    const b = new CellChain()
    const events = [ev(1, { x: 1 }, 1000), ev(2, { x: 2 }, 2000)]
    for (const e of events) { a.append(e); b.append(e) }

    expect(a.detectFork(b)).toBeNull()
  })

  it('no fork when one chain is ahead', () => {
    const a = new CellChain()
    const b = new CellChain()
    const shared = ev(1, { x: 1 }, 1000)
    a.append(shared)
    b.append(shared)
    a.append(ev(2, { x: 2 }, 2000)) // a is ahead

    expect(a.detectFork(b)).toBeNull()
  })

  it('detects fork when chains diverge', () => {
    const a = new CellChain()
    const b = new CellChain()

    // Same first event
    const shared = ev(1, { x: 1 }, 1000)
    a.append(shared)
    b.append(shared)

    // Different second events = fork
    a.append(ev(2, { x: 'from-a' }, 2000))
    b.append(ev(2, { x: 'from-b' }, 2001))

    const fork = a.detectFork(b)
    expect(fork).not.toBeNull()
    expect(fork!.forkAt).toBe(1) // diverges at index 1
  })
})

describe('Merge resolution', () => {
  it('longer chain wins', () => {
    const a = new CellChain()
    const b = new CellChain()

    const shared = ev(1, { x: 1 }, 1000)
    a.append(shared)
    b.append(shared)

    a.append(ev(2, { x: 'a2' }, 2000))
    a.append(ev(3, { x: 'a3' }, 3000))

    b.append(ev(2, { x: 'b2' }, 2001))

    const winner = mergeChains(a, b)
    expect(winner.length()).toBe(3)
    expect(winner.getTip()).toBe(a.getTip())
  })

  it('on tie, earlier timestamp at fork point wins', () => {
    const a = new CellChain()
    const b = new CellChain()

    const shared = ev(1, { x: 1 }, 1000)
    a.append(shared)
    b.append(shared)

    // Same length, a has earlier timestamp at fork
    a.append(ev(2, { x: 'a' }, 2000))
    b.append(ev(2, { x: 'b' }, 2001))

    const winner = mergeChains(a, b)
    expect(winner.getTip()).toBe(a.getTip())
  })

  it('identical chains return either (no fork)', () => {
    const a = new CellChain()
    const b = new CellChain()

    const events = [ev(1, { x: 1 }, 1000), ev(2, { x: 2 }, 2000)]
    for (const e of events) { a.append(e); b.append(e) }

    const winner = mergeChains(a, b)
    expect(winner.getTip()).toBe(a.getTip())
  })
})

describe('Peer convergence with hash chain', () => {
  it('two peers with shared log converge on same chain tip', () => {
    const peerA = new CellChain()
    const peerB = new CellChain()

    // Shared ordered log (e.g., from a message queue)
    const log = [
      ev(1, { status: 'In Cart' }, 1000),
      ev(2, { status: 'Placed' }, 2000),
      ev(3, { status: 'Shipped' }, 3000),
    ]

    for (const e of log) { peerA.append(e); peerB.append(e) }

    expect(peerA.getTip()).toBe(peerB.getTip())
    expect(peerA.verify()).toBe(true)
    expect(peerB.verify()).toBe(true)
  })

  it('peer detecting fork adopts longer chain and replays', () => {
    const peerA = new CellChain()
    const peerB = new CellChain()

    // Shared prefix
    const shared = ev(1, { x: 1 }, 1000)
    peerA.append(shared)
    peerB.append(shared)

    // Peer A has more work
    peerA.append(ev(2, { x: 2 }, 2000))
    peerA.append(ev(3, { x: 3 }, 3000))

    // Peer B has a shorter divergent chain
    peerB.append(ev(2, { x: 'alt' }, 2500))

    // Peer B detects fork and adopts peer A's chain (longer)
    const winner = mergeChains(peerA, peerB)
    expect(winner.length()).toBe(3)
    expect(winner.getTip()).toBe(peerA.getTip())

    // Peer B would rebuild its state by replaying winner's events
    const rebuiltState: Record<string, unknown> = {}
    for (const e of winner.getEvents()) {
      Object.assign(rebuiltState, e.facts)
    }
    expect(rebuiltState.x).toBe(3)
  })
})
