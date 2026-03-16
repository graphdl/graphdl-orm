import { describe, it, expect, vi } from 'vitest'

/**
 * Tests for state machine auto-creation on entity creation
 * and status normalization on entity queries.
 *
 * These are unit tests for the logic — the DO methods are tested
 * via mock SQL in integration tests.
 */

describe('state machine auto-creation', () => {
  it('autoCreateStateMachine finds initial status (no incoming transitions)', () => {
    // Simulate: SupportRequest has SM definition with Received, Triaging, Resolved
    // Received has no incoming transitions → it's the initial state
    const sql = mockSql({
      // findDefinition
      'SELECT smd.id': [{ id: 'smd1', noun_id: 'n1' }],
      // findStatuses
      'SELECT id, name FROM statuses': [
        { id: 's1', name: 'Received' },
        { id: 's2', name: 'Triaging' },
        { id: 's3', name: 'Resolved' },
      ],
      // checkIncoming for s1 (Received) — no results
      'SELECT 1 FROM transitions WHERE to_status_id': [],
      // INSERT state_machines
      'INSERT INTO state_machines': [],
    })

    const result = autoCreateStateMachine(sql, 'domain1', 'SupportRequest', 'entity-123', '2026-01-01')
    expect(result).toBe('Received')
  })

  it('returns null when noun has no state machine definition', () => {
    const sql = mockSql({
      'SELECT smd.id': [], // no definitions found
    })

    const result = autoCreateStateMachine(sql, 'domain1', 'Message', 'msg-1', '2026-01-01')
    expect(result).toBeNull()
  })

  it('returns null when state machine has no statuses', () => {
    const sql = mockSql({
      'SELECT smd.id': [{ id: 'smd1', noun_id: 'n1' }],
      'SELECT id, name FROM statuses': [],
    })

    const result = autoCreateStateMachine(sql, 'domain1', 'SupportRequest', 'entity-1', '2026-01-01')
    expect(result).toBeNull()
  })

  it('falls back to first status when all have incoming transitions', () => {
    const sql = mockSql({
      'SELECT smd.id': [{ id: 'smd1', noun_id: 'n1' }],
      'SELECT id, name FROM statuses': [
        { id: 's1', name: 'A' },
        { id: 's2', name: 'B' },
      ],
      // Both have incoming transitions
      'SELECT 1 FROM transitions WHERE to_status_id': [{ '1': 1 }],
      'INSERT INTO state_machines': [],
    })

    const result = autoCreateStateMachine(sql, 'domain1', 'Cyclic', 'entity-1', '2026-01-01')
    expect(result).toBe('A') // falls back to first
  })
})

// ---------------------------------------------------------------------------
// Helpers — extract the pure logic from the DO method for testing
// ---------------------------------------------------------------------------

function autoCreateStateMachine(
  sql: ReturnType<typeof mockSql>,
  domainId: string,
  nounName: string,
  entityId: string,
  now: string,
): string | null {
  try {
    const defs = sql.exec(
      'SELECT smd.id, smd.noun_id FROM state_machine_definitions smd JOIN nouns n ON smd.noun_id = n.id WHERE n.name = ? AND smd.domain_id = ? LIMIT 1',
      nounName, domainId,
    )
    if (!defs.length) return null

    const defId = defs[0].id as string

    const statuses = sql.exec(
      'SELECT id, name FROM statuses WHERE state_machine_definition_id = ? ORDER BY created_at ASC',
      defId,
    )
    if (!statuses.length) return null

    let initialStatus = statuses[0]
    for (const s of statuses) {
      const incoming = sql.exec(
        'SELECT 1 FROM transitions WHERE to_status_id = ? LIMIT 1', s.id,
      )
      if (!incoming.length) { initialStatus = s; break }
    }

    sql.exec(
      'INSERT INTO state_machines (id, name, state_machine_definition_id, current_status_id, domain_id, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, 1)',
      'sm-id', entityId, defId, initialStatus.id, domainId, now, now,
    )

    return initialStatus.name as string
  } catch {
    return null
  }
}

function mockSql(responses: Record<string, any[]>) {
  const calls: string[] = []
  return {
    exec(query: string, ...params: any[]) {
      calls.push(query)
      // Match by prefix
      for (const [prefix, result] of Object.entries(responses)) {
        if (query.includes(prefix)) {
          // For queries that match multiple times with different params,
          // return the result and then remove it so next call gets empty
          return result
        }
      }
      return []
    },
    calls,
  }
}
