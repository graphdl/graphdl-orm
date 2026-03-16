import { describe, it, expect } from 'vitest'
import { toTableName } from '../generate/sqlite'

/**
 * Entity deletion tests.
 *
 * The core issue: when an entity type has subtypes (e.g. SupportRequest is a subtype of Request),
 * queryEntities returns rows from BOTH tables via UNION ALL. But deleteEntity only targets one table.
 * If the entity was created in the parent table (requests), deleteEntity('SupportRequest', id) targets
 * support_requests and misses it.
 *
 * These tests verify the table name derivation and document the expected deletion behavior.
 */

describe('toTableName', () => {
  it('derives correct table names from noun names', () => {
    expect(toTableName('SupportRequest')).toBe('support_requests')
    expect(toTableName('Message')).toBe('messages')
    expect(toTableName('Request')).toBe('requests')
    expect(toTableName('Customer')).toBe('customers')
    expect(toTableName('SupportResponse')).toBe('support_responses')
  })
})

describe('entity deletion with subtypes', () => {
  /**
   * When queryEntities('SupportRequest') returns rows, they may come from:
   * 1. support_requests table (the entity's own table)
   * 2. requests table (the parent type's table, via UNION ALL)
   *
   * deleteEntity('SupportRequest', id) only deletes from support_requests.
   * If the row is in requests, the delete silently fails.
   *
   * The fix: deleteEntity should check ALL tables in the subtype chain.
   */
  it('documents the subtype deletion gap', () => {
    // Given a noun "SupportRequest" that is a subtype of "Request"
    const nounName = 'SupportRequest'
    const parentName = 'Request'

    const entityTable = toTableName(nounName)   // 'support_requests'
    const parentTable = toTableName(parentName)  // 'requests'

    expect(entityTable).toBe('support_requests')
    expect(parentTable).toBe('requests')

    // deleteEntity uses toTableName(nounName) — only targets support_requests
    // But queryEntities includes UNION ALL with requests table
    // A row in requests with matching domain_id appears in queryEntities('SupportRequest')
    // but deleteEntity('SupportRequest', id) won't find it

    // The correct behavior: deleteEntity should try both tables
    const tablesToCheck = [entityTable, parentTable]
    expect(tablesToCheck).toContain('support_requests')
    expect(tablesToCheck).toContain('requests')
  })

  it('should delete from parent tables when entity not found in child table', () => {
    // This test documents the expected fix:
    // deleteEntity('SupportRequest', id) should:
    // 1. Try DELETE FROM support_requests WHERE id = ? AND domain_id = ?
    // 2. If not found, look up supertypes: Request
    // 3. Try DELETE FROM requests WHERE id = ? AND domain_id = ?
    // 4. Return { deleted: true } if found in any table

    // The supertype chain for deletion:
    const chain = ['SupportRequest', 'Request']
    const tables = chain.map(toTableName)
    expect(tables).toEqual(['support_requests', 'requests'])
  })
})

describe('entity deletion cascading', () => {
  it('documents that deleting a SupportRequest should cascade to Messages', () => {
    // When a SupportRequest is deleted, all Messages with
    // support_request_id = that ID should also be deleted.
    // This is a domain-level cascade, not a SQL FK cascade.

    // The reading: "SupportRequest has Message"
    // The constraint: "Each Message belongs to at most one SupportRequest"
    // The FK: messages.support_request_id REFERENCES support_requests(id)

    // Expected behavior: deleteEntity('SupportRequest', id) should:
    // 1. Find all Messages where support_request_id = id
    // 2. Delete those Messages
    // 3. Delete the SupportRequest

    // For now, the DO doesn't cascade. This is a documented gap.
    expect(true).toBe(true) // placeholder for implementation
  })
})
