import { describe, it, expect } from 'vitest'
import { cellKey, parseCellKey } from './cell-key'

describe('cellKey — RMAP-derived cell naming (#217)', () => {
  it('prefixes the noun type so two entities with the same id across types get disjoint DOs', () => {
    const a = cellKey('Customer', '123')
    const b = cellKey('Order', '123')
    expect(a).toBe('Customer:123')
    expect(b).toBe('Order:123')
    expect(a).not.toBe(b)
  })

  it('passes through unqualified ids when no type is provided (legacy path)', () => {
    // Used for cells that pre-date per-type scoping — e.g. `defs:${slug}`
    // and `domain-secrets:${domain}` keys that already carry their own
    // prefix. Keeping the empty-type fall-through means cellKey()
    // doesn't break those call sites when they migrate gradually.
    expect(cellKey('', 'defs:alpha')).toBe('defs:alpha')
    expect(cellKey('   ', 'global')).toBe('global')
  })

  it('trims whitespace so equivalent inputs shard to the same cell', () => {
    // Definition 2 (Cell Isolation) requires that all writes to one
    // logical cell serialise through one DO. Whitespace drift would
    // silently split the cell's writers across two DOs.
    expect(cellKey('Order', '123')).toBe(cellKey(' Order', '123 '))
    expect(cellKey('Order', '123')).toBe(cellKey('Order\n', '\t123'))
  })

  it('round-trips via parseCellKey for typed keys', () => {
    const key = cellKey('Feature Request', 'FR-042')
    const parsed = parseCellKey(key)
    expect(parsed).toEqual({ nounType: 'Feature Request', entityId: 'FR-042' })
  })

  it('returns null from parseCellKey for legacy raw-uuid keys', () => {
    // A pre-#217 DO name has no colon — parseCellKey must not invent
    // a type, it should report "I don't know" so callers can fall
    // back to a Registry lookup for the noun type.
    expect(parseCellKey('abc-123-def')).toBeNull()
  })

  it('returns null from parseCellKey when the key ends at the separator', () => {
    // "Order:" is malformed — type without id. Treat as legacy.
    expect(parseCellKey('Order:')).toBeNull()
  })

  it('preserves internal colons in the entityId half', () => {
    // Compound reference schemes canonically concatenate role values
    // — those concatenations may themselves contain colons (timestamps,
    // URNs). parseCellKey must split only on the FIRST colon so the
    // entityId remains byte-identical to what cellKey() saw.
    const key = cellKey('Event', '2026-04-18T12:00:00Z')
    expect(key).toBe('Event:2026-04-18T12:00:00Z')
    const parsed = parseCellKey(key)
    expect(parsed).toEqual({ nounType: 'Event', entityId: '2026-04-18T12:00:00Z' })
  })
})
