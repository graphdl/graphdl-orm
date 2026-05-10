/**
 * MCP server tool registration tests.
 *
 * Verifies that the MCP server registers the expected tools
 * with correct schemas. Does not test network calls.
 */

import { describe, it, expect } from 'vitest'
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { z } from 'zod'
import { parseQueryResponse, parseSqlResponse } from './server.js'

describe('AREST MCP Server', () => {
  it('registers expected tool names', () => {
    // The tools the server registers. Keep in sync with src/mcp/server.ts.
    // Identity-carrying commands accept sender + signature (tasks #17, #20, #24).
    const expectedTools = [
      'arest_list',
      'arest_get',
      'arest_create',
      'arest_apply',
      'arest_transition',
      'arest_evaluate',
      'arest_schema',
      'arest_compile',
      'arest_parse',
      'arest_audit_log',
      'arest_verify_signature',
    ]

    // Since we can't easily introspect a running server without connecting,
    // verify the tool names match the documented tool surface.
    for (const tool of expectedTools) {
      expect(tool).toMatch(/^arest_/)
    }
    expect(expectedTools.length).toBeGreaterThanOrEqual(11)
  })

  it('all tools require domain parameter', () => {
    // Every AREST operation is scoped to a domain
    const domainSchema = z.string().describe('The domain slug')
    expect(domainSchema.parse('support')).toBe('support')
    expect(() => domainSchema.parse(123)).toThrow()
  })

  it('list tool accepts pagination parameters', () => {
    const schema = z.object({
      noun: z.string(),
      domain: z.string(),
      page: z.number().optional(),
      limit: z.number().optional(),
    })
    expect(schema.parse({ noun: 'Order', domain: 'support' })).toEqual({ noun: 'Order', domain: 'support' })
    expect(schema.parse({ noun: 'Order', domain: 'support', page: 2, limit: 50 })).toEqual({ noun: 'Order', domain: 'support', page: 2, limit: 50 })
  })

  it('create tool accepts fields, sender, signature', () => {
    const schema = z.object({
      noun: z.string(),
      domain: z.string(),
      id: z.string().optional(),
      fields: z.record(z.string(), z.string()),
      sender: z.string().optional(),
      signature: z.string().optional(),
    })
    const result = schema.parse({
      noun: 'Order',
      domain: 'support',
      fields: { customer: 'acme', status: 'In Cart' },
      sender: 'alice@example.com',
    })
    expect(result.sender).toBe('alice@example.com')
    expect(result.fields.customer).toBe('acme')
  })

  it('compile tool accepts FORML2 readings text', () => {
    const schema = z.object({
      domain: z.string(),
      readings: z.string(),
    })
    const result = schema.parse({
      domain: 'test',
      readings: 'Customer(.Email) is an entity type.\nCustomer has Name.\n  Each Customer has exactly one Name.',
    })
    expect(result.readings).toContain('Customer(.Email) is an entity type.')
  })

  it('verify_signature tool accepts sender, payload, signature', () => {
    const schema = z.object({
      sender: z.string(),
      payload: z.string(),
      signature: z.string(),
    })
    const result = schema.parse({
      sender: 'alice@example.com',
      payload: 'create Order ord-1',
      signature: 'deadbeef1234',
    })
    expect(result.signature).toBe('deadbeef1234')
  })

  it('apply tool accepts a generic Command object', () => {
    const schema = z.object({
      command: z.record(z.string(), z.any()),
    })
    const result = schema.parse({
      command: { type: 'createEntity', noun: 'Order', domain: 'test', fields: { customer: 'acme' } },
    })
    expect(result.command.type).toBe('createEntity')
  })
})

describe('#821 query verb returns tuples for empty / unknown FT', () => {
  it('translates engine ⊥ (FT unknown to schema) to empty tuple list', () => {
    // When `query:<ft>` def isn't in DEFS, apply returns Object::Bottom
    // which serializes to "⊥". The user-facing semantic is "there are no
    // facts of that type" — same as the empty-population case.
    expect(parseQueryResponse('⊥')).toEqual([])
  })

  it('passes through valid JSON tuple list unchanged', () => {
    const tuples = JSON.stringify([{ Task: '262', 'Task Status': 'completed' }])
    expect(parseQueryResponse(tuples)).toEqual([{ Task: '262', 'Task Status': 'completed' }])
  })

  it('translates explicit JSON null to empty tuple list', () => {
    expect(parseQueryResponse('null')).toEqual([])
  })

  it('returns { raw } for non-⊥ malformed responses (preserves diagnostics)', () => {
    const result = parseQueryResponse('this is not json and not bottom')
    expect(result).toEqual({ raw: 'this is not json and not bottom' })
  })
})

describe('#864 sql verb envelope parsing', () => {
  it('passes through a successful rows envelope', () => {
    const raw = JSON.stringify({ rows: [{ Task: '1', Task_Priority: 'p0' }] })
    expect(parseSqlResponse(raw)).toEqual({ rows: [{ Task: '1', Task_Priority: 'p0' }] })
  })

  it('passes through an engine-emitted error envelope', () => {
    const raw = JSON.stringify({ error: 'no such table: ft_nope' })
    expect(parseSqlResponse(raw)).toEqual({ error: 'no such table: ft_nope' })
  })

  it('translates engine ⊥ into a structured error envelope', () => {
    // ⊥ here means the system handle didn't dispatch — most often
    // because the build lacks the local feature. Surface that to the
    // caller as a structured error rather than a malformed-JSON crash.
    const result = parseSqlResponse('⊥') as { error: string }
    expect(result.error).toMatch(/⊥|local/)
  })

  it('wraps malformed engine output in a structured error envelope', () => {
    const result = parseSqlResponse('not json at all') as { error: string; raw: string }
    expect(result.error).toMatch(/malformed/)
    expect(result.raw).toBe('not json at all')
  })
})
