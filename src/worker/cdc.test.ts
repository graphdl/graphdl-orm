import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import type { BatchEntity } from '../batch-wal'
import { buildCdcEvents, formatCdcMessage, type CdcEvent } from './cdc'

describe('cdc', () => {
  // Use fake timers so timestamp is deterministic
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-01-15T10:30:00.000Z'))
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  describe('buildCdcEvents', () => {
    it('produces one event per entity with correct operation', () => {
      const entities: BatchEntity[] = [
        { id: 'e1', type: 'Noun', domain: 'university', data: { name: 'Student' } },
        { id: 'e2', type: 'Role', domain: 'university', data: { name: 'enrolls' } },
        { id: 'e3', type: 'Constraint', domain: 'university', data: { kind: 'UC' } },
      ]

      const events = buildCdcEvents(entities, 'create')

      expect(events).toHaveLength(3)
      expect(events[0].operation).toBe('create')
      expect(events[1].operation).toBe('create')
      expect(events[2].operation).toBe('create')

      expect(events[0].entityId).toBe('e1')
      expect(events[0].type).toBe('Noun')
      expect(events[0].domain).toBe('university')

      expect(events[1].entityId).toBe('e2')
      expect(events[2].entityId).toBe('e3')
    })

    it('create events include data', () => {
      const entities: BatchEntity[] = [
        { id: 'e1', type: 'Noun', domain: 'university', data: { name: 'Student', isIndependent: true } },
      ]

      const events = buildCdcEvents(entities, 'create')

      expect(events[0].data).toEqual({ name: 'Student', isIndependent: true })
    })

    it('update events include data', () => {
      const entities: BatchEntity[] = [
        { id: 'e1', type: 'Noun', domain: 'university', data: { name: 'StudentUpdated' } },
      ]

      const events = buildCdcEvents(entities, 'update')

      expect(events[0].data).toEqual({ name: 'StudentUpdated' })
      expect(events[0].operation).toBe('update')
    })

    it('delete events do not include data', () => {
      const entities: BatchEntity[] = [
        { id: 'e1', type: 'Noun', domain: 'university', data: { name: 'Student' } },
      ]

      const events = buildCdcEvents(entities, 'delete')

      expect(events[0].data).toBeUndefined()
      expect(events[0].operation).toBe('delete')
    })

    it('empty batch produces empty events array', () => {
      const events = buildCdcEvents([], 'create')

      expect(events).toEqual([])
    })

    it('timestamp is in ISO format', () => {
      const entities: BatchEntity[] = [
        { id: 'e1', type: 'Noun', domain: 'test', data: {} },
      ]

      const events = buildCdcEvents(entities, 'create')

      expect(events[0].timestamp).toBe('2026-01-15T10:30:00.000Z')
      // Verify ISO 8601 format with regex
      expect(events[0].timestamp).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/)
    })

    it('all events in a batch share the same timestamp', () => {
      const entities: BatchEntity[] = [
        { id: 'e1', type: 'Noun', domain: 'test', data: {} },
        { id: 'e2', type: 'Role', domain: 'test', data: {} },
        { id: 'e3', type: 'Constraint', domain: 'test', data: {} },
      ]

      const events = buildCdcEvents(entities, 'update')

      const timestamps = events.map(e => e.timestamp)
      expect(new Set(timestamps).size).toBe(1)
    })
  })

  describe('formatCdcMessage', () => {
    it('produces valid JSON with type="cdc"', () => {
      const events: CdcEvent[] = [
        {
          entityId: 'e1',
          type: 'Noun',
          domain: 'university',
          operation: 'create',
          timestamp: '2026-01-15T10:30:00.000Z',
          data: { name: 'Student' },
        },
      ]

      const message = formatCdcMessage(events)
      const parsed = JSON.parse(message)

      expect(parsed.type).toBe('cdc')
      expect(parsed.events).toHaveLength(1)
      expect(parsed.events[0].entityId).toBe('e1')
      expect(parsed.events[0].operation).toBe('create')
    })

    it('formats empty events array', () => {
      const message = formatCdcMessage([])
      const parsed = JSON.parse(message)

      expect(parsed.type).toBe('cdc')
      expect(parsed.events).toEqual([])
    })

    it('formats multiple events', () => {
      const events: CdcEvent[] = [
        {
          entityId: 'e1',
          type: 'Noun',
          domain: 'university',
          operation: 'create',
          timestamp: '2026-01-15T10:30:00.000Z',
          data: { name: 'Student' },
        },
        {
          entityId: 'e2',
          type: 'Role',
          domain: 'university',
          operation: 'delete',
          timestamp: '2026-01-15T10:30:00.000Z',
        },
      ]

      const message = formatCdcMessage(events)
      const parsed = JSON.parse(message)

      expect(parsed.type).toBe('cdc')
      expect(parsed.events).toHaveLength(2)
      expect(parsed.events[0].data).toEqual({ name: 'Student' })
      expect(parsed.events[1].data).toBeUndefined()
    })
  })
})
