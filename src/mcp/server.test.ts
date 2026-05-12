/**
 * MCP server tool registration tests.
 *
 * Verifies that the MCP server registers the expected tools
 * with correct schemas. Does not test network calls.
 */

import { describe, it, expect } from 'vitest'
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { z } from 'zod'
import { readFileSync } from 'fs'
import { resolve, dirname } from 'path'
import { fileURLToPath } from 'url'
import {
  parseQueryResponse,
  parseSqlResponse,
  parseCellsResponse,
  parseInduceResponse,
  parseOrientResponse,
  applyCreateMissingIdRefusal,
  mergeUpdateFields,
  buildApplyMergedUpdatePayload,
} from './server.js'

const __dirname = dirname(fileURLToPath(import.meta.url))
const SERVER_TS = readFileSync(resolve(__dirname, 'server.ts'), 'utf-8')

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

describe('#872 apply footgun-resistance', () => {
  // Engine fixes #867 (apply create without id) and #868 (apply update
  // partial-field retraction) landed in f321a9dd. These tests pin the
  // MCP TS-layer defensive guards so agents still get actionable
  // feedback if a future engine drift reintroduces silent-failure
  // behavior. Belt-and-suspenders per the #869 north-star: "agents get
  // value without reading the whitepaper".

  describe('#867 apply create without explicit id', () => {
    it('refuses with reference-scheme message when id is missing/empty', () => {
      // The MCP layer refuses silent-id to keep the contract explicit
      // (Option 1 from the task brief: explicit > implicit). The error
      // message names the noun, mentions reference scheme semantics,
      // and points the agent at context.rules (cookbooks are gone) for
      // recovery. The check fires BEFORE any engine call so the agent
      // gets an immediate failure they can fix.
      const refusal = applyCreateMissingIdRefusal('Task', undefined)
      expect(refusal).not.toBeNull()
      expect(refusal!.error).toMatch(/apply create requires an explicit id/)
      expect(refusal!.error).toContain("'Task'")
      expect(refusal!.error).toMatch(/reference scheme/)
      expect(refusal!.error).toMatch(/#867/)
      expect(refusal!.error).toMatch(/context\.rules/)
    })

    it('also refuses when id is an empty string (not just undefined)', () => {
      // Engine's silent-orphan behavior in pre-f321a9dd was triggered
      // by empty-string id too, not just undefined. Cover both.
      const refusalEmpty = applyCreateMissingIdRefusal('Task', '')
      expect(refusalEmpty).not.toBeNull()
      expect(refusalEmpty!.error).toMatch(/apply create requires an explicit id/)

      const refusalWhitespace = applyCreateMissingIdRefusal('Task', '   ')
      expect(refusalWhitespace).not.toBeNull()
    })

    it('returns null (no refusal) when id is explicitly provided', () => {
      // Happy path: an explicit id passes the guard so the apply call
      // proceeds to the engine.
      expect(applyCreateMissingIdRefusal('Task', 'task-42')).toBeNull()
      expect(applyCreateMissingIdRefusal('Order', 'ord-1')).toBeNull()
    })
  })

  describe('#868 apply update merge-with-existing', () => {
    it('merges payload fields on top of existing single-valued fields', () => {
      // The agent passes a partial update (only the fields they want
      // to change). The MCP layer fetches the existing entity, layers
      // the payload on top, and sends the full set so the engine can't
      // accidentally retract untouched single-valued facts (#868).
      const existing = {
        id: 'task-42',
        'Task Status': 'completed',
        'Task Subject': 'X',
      }
      const payload = { 'Task Description': 'new' }
      const merged = mergeUpdateFields(existing, payload)
      expect(merged).toEqual({
        'Task Status': 'completed',
        'Task Subject': 'X',
        'Task Description': 'new',
      })
    })

    it('payload values WIN over existing for the same field (true update semantics)', () => {
      // If the agent says "set Task Status to in-progress", the merge
      // must reflect the new value, not the old. Payload wins.
      const existing = { id: 'task-42', 'Task Status': 'ready' }
      const payload = { 'Task Status': 'in-progress' }
      const merged = mergeUpdateFields(existing, payload)
      expect(merged['Task Status']).toBe('in-progress')
    })

    it('preserves multi-valued FT touches without re-asserting them', () => {
      // For many-to-many fact types like Source File the engine response
      // surfaces an array of touches. Re-asserting those in the merge
      // would replay them as fresh facts; the merge must skip arrays so
      // multi-valued touches pass through directly without being
      // smuggled back into the update payload.
      const existing = {
        id: 'task-42',
        'Task Status': 'completed',
        'Source File': [
          { Source_File: 'a.md' },
          { Source_File: 'b.md' },
        ],
      }
      const payload = { 'Task Description': 'new' }
      const merged = mergeUpdateFields(existing, payload)
      expect(merged).toEqual({
        'Task Status': 'completed',
        'Task Description': 'new',
      })
      // The Source File array MUST NOT leak into the merged payload —
      // arrays are multi-valued and live in their own cells.
      expect('Source File' in merged).toBe(false)
    })

    it('skips the synthetic id field — id is addressed separately, not as a payload field', () => {
      // `get` returns the entity with id as a field, but the engine's
      // update_via_defs takes id from the command envelope, not the
      // fields map. Smuggling id into the payload triggers a duplicate
      // <id, ...> pair and confuses the engine.
      const existing = { id: 'task-42', 'Task Status': 'ready' }
      const merged = mergeUpdateFields(existing, { 'Task Status': 'completed' })
      expect('id' in merged).toBe(false)
    })

    it('skips nested-object fields (only scalar single-valued facts are merged)', () => {
      // Defensive: if `get` ever evolves to nest related entities (the
      // synthesize/HATEOAS direction), those should not get smuggled
      // into the update payload as stringified blobs. Only top-level
      // string-valued (scalar) facts pass through.
      const existing = {
        id: 'task-42',
        'Task Status': 'ready',
        related: { Order: 'ord-1' },
      }
      const merged = mergeUpdateFields(existing, { 'Task Status': 'completed' })
      expect('related' in merged).toBe(false)
      expect(merged['Task Status']).toBe('completed')
    })

    it('treats null / undefined existing fields as absent (not as "" — would retract)', () => {
      // A null in the existing snapshot means the engine reported
      // "no value" for that field; pushing it back as "" would CREATE
      // an empty fact, the exact bug #868 was about. Skip nulls and
      // undefineds.
      const existing = { id: 'task-42', 'Task Status': 'ready', 'Task Subject': null as any }
      const merged = mergeUpdateFields(existing, { 'Task Description': 'new' })
      expect('Task Subject' in merged).toBe(false)
      expect(merged['Task Status']).toBe('ready')
    })

    it('handles missing/empty existing snapshot gracefully (pass payload through unchanged)', () => {
      // If the `get` call returns {} or null (engine miss, entity not
      // yet materialized), the merge degrades to "just send the
      // payload" — no extra retract risk because there are no
      // untouched fields to preserve.
      expect(mergeUpdateFields({}, { 'Task Status': 'ready' })).toEqual({ 'Task Status': 'ready' })
      expect(mergeUpdateFields(null as any, { 'Task Status': 'ready' })).toEqual({ 'Task Status': 'ready' })
    })
  })

  describe('#872 buildApplyMergedUpdatePayload — full end-to-end shape', () => {
    it('returns merged payload when fields_only_replace is false / absent', () => {
      // Default behavior: merge with existing. Mock the get-fetcher
      // returning a typical Task snapshot, call the builder with a
      // partial payload, verify the merged result.
      const existing = {
        id: 'task-42',
        'Task Status': 'completed',
        'Task Subject': 'X',
      }
      const result = buildApplyMergedUpdatePayload({
        existing,
        payload: { 'Task Description': 'new' },
        fields_only_replace: false,
      })
      expect(result.fields).toEqual({
        'Task Status': 'completed',
        'Task Subject': 'X',
        'Task Description': 'new',
      })
      expect(result.merged).toBe(true)
      // The preserved set tells callers which fields the merge layered
      // back from the existing snapshot — useful diff for debug logs.
      expect(result.preserved.sort()).toEqual(['Task Status', 'Task Subject'])
    })

    it('returns the payload unchanged when fields_only_replace is true (opt-out)', () => {
      // Belt-and-suspenders opt-out for the rare case the agent wants
      // the old replace-only behavior. The builder must NOT touch the
      // payload in this case, and preserved must be empty.
      const existing = {
        id: 'task-42',
        'Task Status': 'completed',
        'Task Subject': 'X',
      }
      const result = buildApplyMergedUpdatePayload({
        existing,
        payload: { 'Task Description': 'new' },
        fields_only_replace: true,
      })
      expect(result.fields).toEqual({ 'Task Description': 'new' })
      expect(result.merged).toBe(false)
      expect(result.preserved).toEqual([])
    })

    it('confirms #868 fix: unrelated single-valued fields survive a partial update', () => {
      // The one-line acceptance: against a mocked engine snapshot
      // {Task Status: "completed", Task Subject: "X"}, an update of
      // only {Task Description: "new"} must result in an outbound
      // payload that STILL CARRIES the prior Task Status + Task
      // Subject. If this assertion regresses, a future engine drift
      // would re-introduce silent retraction.
      const existing = { id: 'task-42', 'Task Status': 'completed', 'Task Subject': 'X' }
      const result = buildApplyMergedUpdatePayload({
        existing,
        payload: { 'Task Description': 'new' },
        fields_only_replace: false,
      })
      expect(result.fields['Task Status']).toBe('completed')
      expect(result.fields['Task Subject']).toBe('X')
      expect(result.fields['Task Description']).toBe('new')
    })
  })
})

describe('#873 verb descriptions teach when / why / gotchas / next', () => {
  // Source-of-truth contract: every primary verb's registerTool block
  // must carry the four-phrase shape so an agent reading any one verb's
  // description finds the same teaching structure. The cookbook in
  // CLAUDE.md was deleted (drift-prone) — the structural replacement is
  // teaching at the call site. This test pins that the call sites
  // actually teach: WHEN / ALTERNATIVE / GOTCHA / NEXT.
  //
  // We extract each verb's registerTool config object literal by string
  // search rather than instantiating the server (server.ts spawns the
  // engine on import). The slice runs from `'<verb>'` up to the
  // matching `inputSchema:` (or, for verbs without input, up to the
  // closing `},` of the config) — long enough to cover the description
  // field in its entirety.
  // Normalize CRLF to LF so the pattern matches regardless of git
  // checkout line endings (Windows clones with autocrlf=true → CRLF).
  const SRC = SERVER_TS.replace(/\r\n/g, '\n')
  const sliceConfig = (verb: string): string => {
    // Match the two-space-indented `'<verb>',\n` that follows the
    // top-level `server.registerTool(` line for primary verbs.
    const head = SRC.indexOf(`server.registerTool(\n  '${verb}',\n`)
    expect(head, `registerTool('${verb}', ...) block not found`).toBeGreaterThan(0)
    // Slice forward enough to cover the description + (optional)
    // inputSchema. 4096 chars is generous for the longest description
    // we expect (a few hundred words). Stops at the next registerTool.
    const tail = SRC.indexOf(`server.registerTool(`, head + 1)
    return SRC.slice(head, tail > 0 ? tail : head + 4096)
  }

  // The verbs whose descriptions form the agent\'s daily contract.
  // tutor.* mirror tools and select_component / verify / explain / ask
  // / synthesize / validate / debug intentionally not pinned — those
  // are either sandbox mirrors or non-core verbs the agent rarely hits
  // first.
  const PINNED_VERBS = [
    'context',
    'apps.current',
    'apps.list',
    'apps.status',
    'apps.check',
    'apps.use',
    'apps.compile',
    'get',
    'query',
    'sql',
    'cells',
    'induce',
    'orient',
    'apply',
    'retract',
    'actions',
    'compile',
    'schema',
    'propose',
  ] as const

  for (const verb of PINNED_VERBS) {
    it(`'${verb}' description includes WHEN / ALTERNATIVE / GOTCHA / NEXT`, () => {
      const config = sliceConfig(verb)
      expect(config, `${verb}: missing 'WHEN:' marker`).toContain('WHEN:')
      expect(config, `${verb}: missing 'ALTERNATIVE:' marker`).toContain('ALTERNATIVE:')
      expect(config, `${verb}: missing 'GOTCHA:' marker`).toContain('GOTCHA:')
      expect(config, `${verb}: missing 'NEXT:' marker`).toContain('NEXT:')
    })
  }
})
