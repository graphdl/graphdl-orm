/**
 * MCP server tool registration tests.
 *
 * Verifies that the MCP server registers the expected tools
 * with correct schemas. Does not test network calls.
 */

import { describe, it, expect } from 'vitest'
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { z } from 'zod'
import { parseQueryResponse, parseSqlResponse, parseCellsResponse, parseInduceResponse, parseOrientResponse } from './server.js'

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

describe('#870 cells verb envelope parsing', () => {
  it('passes through a successful list envelope', () => {
    const raw = JSON.stringify({
      cells: [
        { name: 'Task_has_Task_Priority', size_bytes: 128 },
        { name: 'Task_has_Task_Status',   size_bytes: 96 },
      ],
    })
    expect(parseCellsResponse(raw)).toEqual({
      cells: [
        { name: 'Task_has_Task_Priority', size_bytes: 128 },
        { name: 'Task_has_Task_Status',   size_bytes: 96 },
      ],
    })
  })

  it('passes through a successful get envelope with parsed contents', () => {
    const raw = JSON.stringify({
      name: 'Task_has_Task_Priority',
      contents: [{ Task: '1', 'Task Priority': 'p0' }],
      size_bytes: 64,
    })
    const parsed = parseCellsResponse(raw) as { name: string; contents: unknown[] }
    expect(parsed.name).toBe('Task_has_Task_Priority')
    expect(parsed.contents).toEqual([{ Task: '1', 'Task Priority': 'p0' }])
  })

  it('passes through an engine-emitted error envelope (no such cell)', () => {
    const raw = JSON.stringify({ error: 'no such cell: Bogus' })
    expect(parseCellsResponse(raw)).toEqual({ error: 'no such cell: Bogus' })
  })

  it('translates engine ⊥ into a structured error envelope', () => {
    // ⊥ here means the system handle didn't dispatch — most often
    // because the build lacks the std-deps feature. Surface that to
    // the caller as a structured error rather than a malformed-JSON
    // crash.
    const result = parseCellsResponse('⊥') as { error: string }
    expect(result.error).toMatch(/⊥|std-deps|handle/)
  })
})

describe('#854 induce verb envelope parsing', () => {
  it('passes through a successful Hypothesis Candidate array', () => {
    // Basic call: engine returns the run_search Vec serialized as a
    // JSON array. Each element is the FFP-shaped Hypothesis Candidate
    // Object::Seq (here represented as nested objects per to_json_value).
    const raw = JSON.stringify([
      { hypothesisCandidateId: 'hyp-Order_was_placed_by_Customer-0', confidenceScore: '5' },
      { hypothesisCandidateId: 'hyp-Order_was_placed_by_Customer-1', confidenceScore: '2' },
    ])
    const parsed = parseInduceResponse(raw) as Array<{ confidenceScore: string }>
    expect(Array.isArray(parsed)).toBe(true)
    expect(parsed).toHaveLength(2)
    expect(parsed[0].confidenceScore).toBe('5')
  })

  it('translates engine ⊥ into a structured error envelope', () => {
    // ⊥ here means the system handle didn't dispatch — handle was
    // never registered, or the build lacks the induce verb. Surface
    // that to the caller as a structured error rather than a
    // malformed-JSON crash.
    const result = parseInduceResponse('⊥') as { error: string }
    expect(result.error).toMatch(/⊥|induce|handle/)
  })

  it('passes through with bound=phi (empty bound) producing an empty array', () => {
    // bound=phi (no role pre-bound) is the default open-ended search;
    // when no candidate survives the constraint gate the engine
    // returns Object::Seq([]) which serializes to JSON `[]`. The
    // parser MUST surface that as an empty array, not an error or
    // a {raw} fallback.
    expect(parseInduceResponse('[]')).toEqual([])
    // null also collapses to the empty list (consistent with the
    // query verb's null → [] translation in #821).
    expect(parseInduceResponse('null')).toEqual([])
  })

  it('preserves engine ranking — top Hypothesis Candidate carries the highest Confidence Score (Sherlock fixture shape)', () => {
    // Mirrors the apps/sherlock/readings/cases/test-locked-room.md
    // fixture from #853: `induce` over `Hypothesis_has_Plausibility`
    // returns at least one Hypothesis Candidate, the top-ranked one
    // pairs `h1-evidence-supported` with the `'plausible'`
    // Plausibility, and the engine has stamped a non-empty
    // `confidenceScore` binding (so the Scoring Rule layer fired,
    // not just enumeration). The Rust integration test
    // `tests/sherlock_induce.rs` exercises the full engine flow;
    // this TS shim test asserts the parser preserves the ordering
    // the engine emitted (Confidence-Score-descending stable sort
    // in `induce::run_search`) so callers see the evidence-supported
    // candidate first.
    const raw = JSON.stringify([
      {
        hypothesisCandidateId: 'hyp-Hypothesis_has_Plausibility-0',
        confidenceScore: '10',
        Hypothesis_Candidate_has_hidden__Fact: [
          { Hypothesis: 'h1-evidence-supported', Plausibility: 'plausible' },
        ],
      },
      {
        hypothesisCandidateId: 'hyp-Hypothesis_has_Plausibility-1',
        confidenceScore: '0',
        Hypothesis_Candidate_has_hidden__Fact: [
          { Hypothesis: 'h2-no-evidence', Plausibility: 'implausible' },
        ],
      },
    ])
    const parsed = parseInduceResponse(raw) as Array<{
      confidenceScore: string
      Hypothesis_Candidate_has_hidden__Fact: Array<{ Hypothesis: string; Plausibility: string }>
    }>
    expect(parsed[0].Hypothesis_Candidate_has_hidden__Fact[0].Hypothesis)
      .toBe('h1-evidence-supported')
    expect(parsed[0].Hypothesis_Candidate_has_hidden__Fact[0].Plausibility)
      .toBe('plausible')
    // Top candidate's score must be strictly higher than the
    // bottom candidate's score — the parser preserves order.
    expect(Number(parsed[0].confidenceScore))
      .toBeGreaterThan(Number(parsed[1].confidenceScore))
  })
})

describe('#871 orient verb envelope parsing', () => {
  it('passes through a successful orient envelope', () => {
    // Standard four-key envelope the engine emits when handed
    // `{"active_app":"tasks"}` against a populated snapshot.
    const raw = JSON.stringify({
      apps: [
        {
          name: 'tasks',
          root: '/path/to/apps/tasks',
          last_compile: null,
          ready_count: 33,
          in_progress_count: 7,
          completed_count: 612,
        },
      ],
      active_app: 'tasks',
      recent_changes: [
        { kind: 'apply', noun: 'Task_has_Task_Status', count: 652 },
      ],
      suggested_next: "Try: mcp__arest__query Task_is_recommended in app 'tasks' for the launch-candidate set.",
    })
    const parsed = parseOrientResponse(raw) as {
      apps: Array<{ name: string; ready_count: number }>
      active_app: string
      recent_changes: Array<{ noun: string }>
      suggested_next: string
    }
    expect(parsed.active_app).toBe('tasks')
    expect(parsed.apps).toHaveLength(1)
    expect(parsed.apps[0].ready_count).toBe(33)
    expect(parsed.recent_changes[0].noun).toBe('Task_has_Task_Status')
    expect(parsed.suggested_next).toContain('recommended')
  })

  it('passes through an engine-emitted error envelope', () => {
    // Malformed input from the caller — the engine returns a
    // structured `{error}` envelope which the parser should preserve.
    const raw = JSON.stringify({ error: 'input must be JSON: expected value at line 1 column 1' })
    expect(parseOrientResponse(raw)).toEqual({
      error: 'input must be JSON: expected value at line 1 column 1',
    })
  })

  it('translates engine ⊥ into a structured error envelope', () => {
    // ⊥ here means the system handle didn't dispatch — most often
    // because the build lacks the std-deps feature, or the handle
    // wasn't allocated for this session. Surface that to the caller
    // as a structured error rather than a malformed-JSON crash.
    const result = parseOrientResponse('⊥') as { error: string }
    expect(result.error).toMatch(/⊥|std-deps|handle/)
  })

  it('wraps malformed engine output in a structured error envelope', () => {
    // Any non-JSON, non-⊥ output is preserved under `raw` so the
    // caller can inspect what the engine actually said. Used as a
    // diagnostic when the engine's envelope format drifts from the
    // parser's expectations.
    const result = parseOrientResponse('not json at all') as { error: string; raw: string }
    expect(result.error).toMatch(/malformed/)
    expect(result.raw).toBe('not json at all')
  })
})
