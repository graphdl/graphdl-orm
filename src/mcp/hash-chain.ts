/**
 * Hash-linked event chain for peer-to-peer cell convergence.
 *
 * Each cell event includes the hash of the previous event. This gives
 * total ordering per cell, tamper detection, and deterministic fork
 * resolution without a central authority.
 *
 * This is Git's model applied to cell events, not Bitcoin's. No
 * proof-of-work. Known peers, deterministic merge rule.
 */

import { type CellEvent } from './streaming'

export interface ChainedEvent extends CellEvent {
  /** Hash of this event (computed from contents + prevHash) */
  hash: string
  /** Hash of the previous event in this cell's chain */
  prevHash: string
}

/** Deterministic hash from event contents. Uses a simple string hash
 *  for testability. Production would use SHA-256. */
export function eventHash(event: CellEvent, prevHash: string): string {
  const content = `${prevHash}:${event.sequence}:${event.noun}:${event.entityId}:${event.operation}:${JSON.stringify(event.facts)}:${event.timestamp}`
  let hash = 0
  for (let i = 0; i < content.length; i++) {
    const ch = content.charCodeAt(i)
    hash = ((hash << 5) - hash) + ch
    hash |= 0
  }
  return Math.abs(hash).toString(36)
}

/** A hash-linked event log for one cell. */
export class CellChain {
  private events: ChainedEvent[] = []
  private tip: string = '0' // genesis

  /** Append an event to the chain. Returns the chained event with hash. */
  append(event: CellEvent): ChainedEvent {
    const hash = eventHash(event, this.tip)
    const chained: ChainedEvent = { ...event, hash, prevHash: this.tip }
    this.events.push(chained)
    this.tip = hash
    return chained
  }

  /** Get the chain tip hash. */
  getTip(): string { return this.tip }

  /** Get chain length. */
  length(): number { return this.events.length }

  /** Get all events in order. */
  getEvents(): ChainedEvent[] { return [...this.events] }

  /** Verify chain integrity: each event's prevHash matches the previous event's hash. */
  verify(): boolean {
    let expectedPrev = '0'
    for (const event of this.events) {
      if (event.prevHash !== expectedPrev) return false
      const computed = eventHash(event, event.prevHash)
      if (event.hash !== computed) return false
      expectedPrev = event.hash
    }
    return true
  }

  /** Detect fork: returns true if another chain diverges from this one. */
  detectFork(other: CellChain): { forkAt: number; thisLength: number; otherLength: number } | null {
    const thisEvents = this.events
    const otherEvents = other.getEvents()
    const minLen = Math.min(thisEvents.length, otherEvents.length)

    for (let i = 0; i < minLen; i++) {
      if (thisEvents[i].hash !== otherEvents[i].hash) {
        return { forkAt: i, thisLength: thisEvents.length, otherLength: otherEvents.length }
      }
    }

    // No fork in the common prefix. One may be longer.
    if (thisEvents.length !== otherEvents.length) {
      return null // not a fork, one is just ahead
    }
    return null // identical chains
  }
}

/** Deterministic merge: longest chain wins. On tie, earlier timestamp at fork point. */
export function mergeChains(a: CellChain, b: CellChain): CellChain {
  const fork = a.detectFork(b)
  if (!fork) {
    // No fork: return the longer one (or either if equal)
    return a.length() >= b.length() ? a : b
  }

  // Fork detected: longest chain wins
  if (fork.thisLength > fork.otherLength) return a
  if (fork.otherLength > fork.thisLength) return b

  // Same length: earlier timestamp at fork point wins
  const aEvents = a.getEvents()
  const bEvents = b.getEvents()
  return aEvents[fork.forkAt].timestamp <= bEvents[fork.forkAt].timestamp ? a : b
}
