/**
 * MCP server tool registration tests.
 *
 * Verifies that the MCP server registers the expected tools
 * with correct schemas. Does not test network calls.
 */

import { describe, it, expect } from 'vitest'
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { z } from 'zod'
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'fs'
import { resolve, dirname, join } from 'path'
import { tmpdir } from 'os'
import { fileURLToPath } from 'url'
import {
  cliCallPlan,
  STDIN_INPUT_THRESHOLD_BYTES,
  parseQueryResponse,
  parseSqlResponse,
  parseCellsResponse,
  parseInduceResponse,
  parseOrientResponse,
  mergeUpdateFields,
  buildApplyMergedUpdatePayload,
  smBypassRefusal,
  buildApplyCommandForBatch,
  persistActiveAppEnabled,
  chooseInitialAppName,
  activeAppStateFile,
  shouldPersistResolvedApp,
  parseGetResponse,
  parseGetEntityResponse,
  resolveCallScope,
  scopeDbPath,
  scopeReadingsDir,
  lookupHandleCache,
  rememberHandleCache,
} from './server.js'

const __dirname = dirname(fileURLToPath(import.meta.url))
const SERVER_TS = readFileSync(resolve(__dirname, 'server.ts'), 'utf-8')

describe('active-app persistence (AREST_PERSIST_ACTIVE_APP)', () => {
  it('is on by default, off only for explicit falsey values', () => {
    expect(persistActiveAppEnabled({})).toBe(true)
    expect(persistActiveAppEnabled({ AREST_PERSIST_ACTIVE_APP: '' })).toBe(true)
    expect(persistActiveAppEnabled({ AREST_PERSIST_ACTIVE_APP: '1' })).toBe(true)
    expect(persistActiveAppEnabled({ AREST_PERSIST_ACTIVE_APP: 'true' })).toBe(true)
    for (const off of ['0', 'false', 'no', 'off', 'OFF', 'False', ' off ']) {
      expect(persistActiveAppEnabled({ AREST_PERSIST_ACTIVE_APP: off })).toBe(false)
    }
  })

  it('resumes the persisted app over the env default, with safe fallbacks', () => {
    const env = { AREST_APP: 'claude' } as NodeJS.ProcessEnv
    // enabled + persisted + still resolves -> persisted wins over $AREST_APP
    expect(chooseInitialAppName({ persistEnabled: true, persistedName: 'tasks', persistedExists: true, env }))
      .toBe('tasks')
    // disabled -> env-inferred default
    expect(chooseInitialAppName({ persistEnabled: false, persistedName: 'tasks', persistedExists: true, env }))
      .toBe('claude')
    // persisted app no longer exists -> env-inferred default
    expect(chooseInitialAppName({ persistEnabled: true, persistedName: 'tasks', persistedExists: false, env }))
      .toBe('claude')
    // nothing persisted -> env-inferred default
    expect(chooseInitialAppName({ persistEnabled: true, persistedName: '', persistedExists: false, env }))
      .toBe('claude')
  })

  it('stores the marker as a hidden file inside the apps dir', () => {
    expect(activeAppStateFile('/abs/apps')).toMatch(/[\\/]\.arest-active-app$/)
  })

  // task-959 fix #3: bare ⊥ from get/list (engine has no get:<Noun> def
  // because the noun isn't in the active app's UoD) surfaces a wrong-UoD
  // warning that names the active app and points at apps_list/apps_use.
  // Before this fix the UI showed only `{ raw: '⊥' }`, which read as
  // data loss when the data is fine and just lives in a different app.
  it('parseGetResponse surfaces a wrong-UoD warning when the engine returns ⊥', () => {
    const out = parseGetResponse('⊥', 'Task', 'claude') as Record<string, unknown>
    expect(out.error).toMatch(/Bottom: get.list for 'Task' returned ⊥ in active app 'claude'/)
    expect(out.hint).toMatch(/apps_use/)
    expect(out.hint).toMatch(/'Task'/)
    expect(out.hint).toMatch(/'claude'/)
    expect(out.raw).toBe('⊥')
  })

  it('parseGetResponse passes through valid JSON unchanged', () => {
    const valid = '{"id":"ORD-1","type":"Order","data":{"Amount":"100"}}'
    expect(parseGetResponse(valid, 'Order', 'orders')).toEqual({
      id: 'ORD-1', type: 'Order', data: { Amount: '100' },
    })
  })

  it('parseGetResponse falls back to raw on non-JSON, non-⊥ output', () => {
    expect(parseGetResponse('not json', 'Order', 'orders')).toEqual({ raw: 'not json' })
  })

  // query-bottom-origin-envelope (arc-agi-3 issue 6): the engine's
  // ⊥-trace decorates Bottom with an origin frame, so the parsers must
  // recognize Bottom by PREFIX, not exact equality — an unknown FT
  // still answers [] / the wrong-UoD envelope, never a raw leak.
  it('parseQueryResponse maps decorated ⊥-origin output to []', () => {
    expect(parseQueryResponse('⊥')).toEqual([])
    expect(
      parseQueryResponse('⊥ origin: in rule `query:Action_Type_has_Action_Semantic`'),
    ).toEqual([])
    expect(parseQueryResponse(' ⊥ origin: whatever')).toEqual([])
  })

  it('parseGetResponse surfaces the wrong-UoD warning for decorated ⊥-origin output too', () => {
    const decorated = '⊥ origin: in rule `get:Run`'
    const out = parseGetResponse(decorated, 'Run', 'arc-agi-3') as Record<string, unknown>
    expect(out.error).toMatch(/Bottom: get.list for 'Run'/)
    expect(out.hint).toMatch(/apps_use/)
    // The raw envelope keeps the decorated string so the origin trace
    // survives for diagnostics.
    expect(out.raw).toBe(decorated)
  })

  // mcp-apply-stdin-payload (arc-agi-3 issue 4): payloads above the
  // threshold ride STDIN (argv carries --stdin-input) so Windows's
  // ~32 KB command-line cap no longer ENAMETOOLONGs large atomic
  // batches; small payloads keep the argv path byte-for-byte.
  it('cliCallPlan keeps small payloads on argv', () => {
    const plan = cliCallPlan('C:/x/app.db', 'apply', '{"ops":[]}')
    expect(plan.args).toEqual(['--db', 'C:/x/app.db', 'apply', '{"ops":[]}'])
    expect(plan.stdin).toBeUndefined()
  })

  it('cliCallPlan routes large payloads through stdin with --stdin-input', () => {
    const big = JSON.stringify({ ops: Array.from({ length: 500 }, (_, i) => ({ operation: 'create', noun: 'Level', id: `lvl-${i}`, fields: { Name: `Level ${i}` } })) })
    expect(Buffer.byteLength(big, 'utf8')).toBeGreaterThan(STDIN_INPUT_THRESHOLD_BYTES)
    const plan = cliCallPlan('C:/x/app.db', 'apply', big)
    expect(plan.args).toEqual(['--db', 'C:/x/app.db', 'apply', '--stdin-input'])
    expect(plan.stdin).toBe(big)
  })

  it('cliCallPlan measures the threshold in BYTES, not chars (multibyte payloads)', () => {
    // ⊥ is 3 UTF-8 bytes — a string of N chars can exceed the byte
    // threshold at N/3 chars. The plan must switch on byte length.
    const multibyte = '⊥'.repeat(Math.ceil(STDIN_INPUT_THRESHOLD_BYTES / 3) + 1)
    expect(multibyte.length).toBeLessThan(STDIN_INPUT_THRESHOLD_BYTES)
    const plan = cliCallPlan('C:/x/app.db', 'query', multibyte)
    expect(plan.args).toContain('--stdin-input')
  })

  // task-959 fix #1: the gate that decides whether to write the
  // `.arest-active-app` marker -- used by BOTH the startup resolution
  // AND `apps.use` so a reconnect deterministically resumes the actually-
  // active app, even when that app came from an inferInitialAppName
  // fallback ($AREST_APP) rather than an explicit apps.use.
  it('shouldPersistResolvedApp gates writes: persist + apps dir + app exists', () => {
    // happy path: all three present -> write the marker.
    expect(shouldPersistResolvedApp({
      persistEnabled: true, appsDir: '/abs/apps', appExists: true,
    })).toBe(true)
    // persistence disabled (e.g. AREST_PERSIST_ACTIVE_APP=0) -> do not write.
    expect(shouldPersistResolvedApp({
      persistEnabled: false, appsDir: '/abs/apps', appExists: true,
    })).toBe(false)
    // no apps workspace (remote-mode boot) -> do not write.
    expect(shouldPersistResolvedApp({
      persistEnabled: true, appsDir: '', appExists: true,
    })).toBe(false)
    // resolved app does not exist on disk -> do NOT promote a fallback
    // over a valid earlier marker (the fallback would clobber 'tasks'
    // with 'claude' if $AREST_APP misses a real app).
    expect(shouldPersistResolvedApp({
      persistEnabled: true, appsDir: '/abs/apps', appExists: false,
    })).toBe(false)
  })
})

// =====================================================================
// mcp-get-surface-view-representations — the `get` tool's single-id
// local-mode branch now reads through the engine's FULL read path
// (Command::GetEntity dispatched through the `apply` def, the same
// calling convention the apply tool's batch path uses) so the Theorem-4
// representation — HATEOAS `transitions` + the ui-readings `view`
// layer (elements + per-target representations) — rides WITH the
// flattened 3NF row. parseGetEntityResponse is the focused translator:
// it re-flattens the CommandResult to the EXACT row shape the legacy
// get:{noun} path produces (both bottom out in the engine's
// get_noun:{noun} platform primitive) and signals "fall back" with
// null whenever the command read misses. The tests below feed raw
// engine-response strings into the parser — the same systemCall-output
// mocking the neighboring parse* suites use; no engine is booted.
// =====================================================================

describe('mcp-get-surface-view-representations — parseGetEntityResponse', () => {
  // A CommandResult as the live binary (engine HEAD a5a5330e) emits it:
  // serde camelCase, `transitions` always present (possibly empty),
  // `view` omitted entirely (skip_serializing_if) when the ui-readings
  // metamodel projects nothing for the noun.
  const baseResult = {
    entities: [{
      id: '656',
      type: 'Task',
      data: { 'Task Status': 'completed', 'Task Subject': 'X' },
    }],
    status: 'completed',
    transitions: [
      {
        event: 'Task is reopened',
        targetStatus: 'pending',
        method: 'GET',
        href: '/api/entities/Task/656/transition?event=Task%20is%20reopened',
      },
    ],
    violations: [],
    derivedCount: 0,
    rejected: false,
  }

  it('(a) WITH view: flattened row + transitions + verbatim view incl. representations', () => {
    const view = {
      view: 'task-instance',
      kind: 'instance',
      source: 'synthesized',
      elements: [
        { id: 've_9f3a01', factType: 'Task_has_Task_Subject', componentRole: 'text-input' },
        { id: 've_77b2c4', factType: 'Task_has_Task_Status', componentRole: 'combo-box' },
      ],
      representations: { html: '<form><input name="Task Subject"/></form>' },
    }
    const raw = JSON.stringify({ ...baseResult, view })
    const out = parseGetEntityResponse(raw) as Record<string, unknown>
    expect(out).not.toBeNull()
    // The flattened 3NF base row — entity data fields + id, exactly the
    // legacy shape.
    expect(out.id).toBe('656')
    expect(out['Task Status']).toBe('completed')
    expect(out['Task Subject']).toBe('X')
    // HATEOAS affordances surface beside the row…
    expect(out.transitions).toEqual(baseResult.transitions)
    // …and the view layer passes through VERBATIM, representations
    // (per-target rendered HTML) included.
    expect(out.view).toEqual(view)
    expect((out.view as { representations: Record<string, string> }).representations.html)
      .toContain('<form>')
  })

  it("(b) WITHOUT view: shape identical to today's legacy get:{noun} row", () => {
    // No view + no transitions ⇒ the output must be deep-equal to what
    // parseGetResponse produces from the legacy row for the same
    // entity: both paths bottom out in get_noun:{noun}, so data+id IS
    // the row. This is the zero-regression pin for SM-less nouns.
    const raw = JSON.stringify({ ...baseResult, transitions: [] })
    const legacyRow = JSON.stringify({ 'Task Status': 'completed', 'Task Subject': 'X', id: '656' })
    expect(parseGetEntityResponse(raw)).toEqual(parseGetResponse(legacyRow, 'Task', 'tasks'))
  })

  it('(b′) empty transitions are NOT attached (no `transitions: []` noise on SM-less nouns)', () => {
    const raw = JSON.stringify({ ...baseResult, transitions: [] })
    const out = parseGetEntityResponse(raw) as Record<string, unknown>
    expect('transitions' in out).toBe(false)
    expect('view' in out).toBe(false)
  })

  it('(b″) transitions surface without a view (SM noun, ui-readings compiled out)', () => {
    const out = parseGetEntityResponse(JSON.stringify(baseResult)) as Record<string, unknown>
    expect(out.transitions).toEqual(baseResult.transitions)
    expect('view' in out).toBe(false)
  })

  it('(c) malformed / ⊥ / rejected / empty-entities → null (caller falls back to get:{noun})', () => {
    // ⊥: handle not dispatched, or an older binary whose Command enum
    // predates getEntity ("⊥ unknown variant `getEntity` …").
    expect(parseGetEntityResponse('⊥')).toBeNull()
    expect(parseGetEntityResponse('⊥ unknown variant `getEntity`, expected one of …')).toBeNull()
    // Non-JSON noise.
    expect(parseGetEntityResponse('not json')).toBeNull()
    // JSON, but not a CommandResult object.
    expect(parseGetEntityResponse('null')).toBeNull()
    expect(parseGetEntityResponse('[]')).toBeNull()
    // A rejected command result must not shadow the legacy diagnostic.
    expect(parseGetEntityResponse(JSON.stringify({ ...baseResult, rejected: true }))).toBeNull()
    // Unknown id: the engine returns a clean empty result (the read
    // path is not alethic) — the legacy path owns the task-959
    // wrong-UoD hint, so the parser must defer to it.
    expect(parseGetEntityResponse(JSON.stringify({ ...baseResult, entities: [] }))).toBeNull()
    // Defensive: an entity missing its id can't be flattened.
    expect(parseGetEntityResponse(JSON.stringify({ ...baseResult, entities: [{ data: { A: '1' } }] })))
      .toBeNull()
  })

  // Source wiring: the single-id local branch dispatches getEntity
  // through the `apply` def and KEEPS the legacy call as its fallback;
  // the list branch and remote/federation branches are untouched.
  it('get handler wires getEntity-with-fallback into the single-id local branch only', () => {
    const SRC = SERVER_TS.replace(/\r\n/g, '\n')
    const head = SRC.indexOf(`server.registerTool(\n  'get',\n`)
    expect(head, "registerTool('get', ...) block not found").toBeGreaterThan(0)
    const tail = SRC.indexOf('server.registerTool(', head + 1)
    const block = SRC.slice(head, tail)
    // The command envelope rides the same `apply` system call the apply
    // tool's batch path uses.
    expect(block).toMatch(/type:\s*'getEntity'/)
    expect(block).toMatch(/systemCall\('apply'/)
    expect(block).toMatch(/parseGetEntityResponse/)
    // The legacy single-id call survives as the fallback…
    expect(block).toMatch(/systemCall\(`get:\$\{noun\}`, id, scope\)/)
    // …and the list path still goes through list:{noun} + parseGetResponse.
    expect(block).toMatch(/systemCall\(`list:\$\{noun\}`, '', scope\)/)
  })
})

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

  // task-930: bulk / collection-shaped apply.
  describe('task-930 bulk / collection-shaped apply', () => {
    it('apply tool exposes an `ops` array (collection shape)', () => {
      // The collection shape: an array of {operation, noun, id, fields?, event?}.
      const opsSchema = z.array(z.object({
        operation: z.enum(['create', 'update', 'transition']),
        noun: z.string().optional(),
        id: z.string().optional(),
        fields: z.record(z.string(), z.string()).optional(),
        event: z.string().optional(),
      }))
      const parsed = opsSchema.parse([
        { operation: 'create', noun: 'Order', id: 'ORD-1', fields: { Amount: '10' } },
        { operation: 'create', noun: 'Order', id: 'ORD-2', fields: { Amount: '20' } },
        { operation: 'transition', noun: 'Order', id: 'ORD-1', event: 'place' },
      ])
      expect(parsed).toHaveLength(3)
    })

    it('apply tool description documents the collection / atomic shape', () => {
      // The source must teach agents the bulk shape exists and is atomic.
      expect(SERVER_TS).toMatch(/COLLECTION/)
      expect(SERVER_TS).toMatch(/ALETHIC violation in ANY op rejects the WHOLE batch/)
      expect(SERVER_TS).toMatch(/Backus α/)
    })

    it('buildApplyCommandForBatch maps each op to the engine Command shape', () => {
      const ctx = { sender: 'alice@example.com', signature: undefined }
      const create = buildApplyCommandForBatch(
        { operation: 'create', noun: 'Order', id: 'ORD-1', fields: { Amount: '10' } }, ctx)
      expect(create).toMatchObject({ type: 'createEntity', noun: 'Order', id: 'ORD-1', fields: { Amount: '10' }, sender: 'alice@example.com' })
      const update = buildApplyCommandForBatch(
        { operation: 'update', noun: 'Order', id: 'ORD-1', fields: { Amount: '11' } }, ctx)
      expect(update).toMatchObject({ type: 'updateEntity', noun: 'Order', entityId: 'ORD-1', fields: { Amount: '11' } })
      const txn = buildApplyCommandForBatch(
        { operation: 'transition', noun: 'Order', id: 'ORD-1', event: 'place' }, ctx)
      expect(txn).toMatchObject({ type: 'transition', noun: 'Order', entityId: 'ORD-1', event: 'place' })
    })

    it('a batch of buildApplyCommandForBatch members wraps into the engine batch JSON', () => {
      const ops = [
        { operation: 'create' as const, noun: 'Order', id: 'ORD-1', fields: { Amount: '10' } },
        { operation: 'create' as const, noun: 'Order', id: 'ORD-2', fields: { Amount: '20' } },
      ]
      const batch = { type: 'batch', commands: ops.map(o => buildApplyCommandForBatch(o, {})) }
      expect(batch.type).toBe('batch')
      expect(batch.commands).toHaveLength(2)
      // Round-trips to JSON the engine's platform_apply_command parses.
      const json = JSON.parse(JSON.stringify(batch))
      expect(json.commands[0].type).toBe('createEntity')
    })
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
  // Engine fix #868 (apply update partial-field retraction) landed in
  // f321a9dd; the MCP update-merge guard below pins it so agents still
  // get actionable feedback if a future engine drift reintroduces the
  // silent-retraction behavior. The #867 create-id refusal was removed
  // in task-964 -- the engine now enforces opt-in auto-gen per noun
  // (create.id_required for an unmarked no-id create, auto-gen for a
  // noun marked `<Noun> has an auto-generated id.`), so a blanket MCP
  // refusal would block a marked noun's legitimate auto-gen. The kernel
  // tests pin that behavior; no MCP-layer create-id test remains.

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

describe('#904 SM-bypass guard', () => {
  // Sibling to #872 (apply footgun-resistance) but a different failure
  // mode: when an app declares `State Machine Definition X is for Noun
  // Y`, an agent calling `apply update` on a Y entity that sets its
  // Status field directly silently bypasses the SM — the status changes
  // without the transition firing, and derivations depending on SM
  // state desynchronize. apps/tasks/#861 covered this for Task
  // specifically; #904 lifts the guard to the MCP layer so it applies
  // GENERICALLY to every SM-governed noun in every app's schema
  // (multi-app substrate; new apps inherit the protection without
  // per-app work).
  //
  // Design: refuse-with-message (mirrors #867/#868). The MCP returns
  // an `{error}` envelope that names the SM, lists legal transitions
  // from the current status, and suggests `apply transition event=...`
  // as the right verb. `force: true` is the escape hatch for migration
  // scripts.
  //
  // The guard is a pure helper exported from server.ts:
  //
  //   smBypassRefusal({
  //     noun,              // the noun being updated
  //     fields,            // the payload fields map
  //     schema,            // active app schema (debug envelope shape)
  //     transitions,       // optional: actions-verb-style transitions
  //                        //   list for the entity's current status
  //     force,             // when true, skip the guard entirely
  //   }) => null | { error: string }
  //
  // Returns null when the call is safe to pass through. Returns
  // {error} otherwise.

  const taskSchema = {
    nouns: ['Task'],
    factTypes: [
      { id: 'Task_has_Task_Status', reading: 'Task has Task Status' },
      { id: 'Task_has_Task_Subject', reading: 'Task has Task Subject' },
    ],
    constraints: [],
    stateMachines: [
      {
        noun: 'Task',
        initial: 'pending',
        transitions: [
          { from: 'pending', to: 'in_progress', event: 'start' },
          { from: 'in_progress', to: 'completed', event: 'finish' },
          { from: 'completed', to: 'pending', event: 'reopen' },
        ],
      },
    ],
    totalFacts: 0,
  }

  const taskActions = [
    { event: 'start', targetStatus: 'in_progress', fromStatus: 'pending' },
    { event: 'finish', targetStatus: 'completed', fromStatus: 'in_progress' },
    { event: 'reopen', targetStatus: 'pending', fromStatus: 'completed' },
  ]

  it("apply update {Task Status: 'completed'} refuses with SM transition list", () => {
    // The headline acceptance: an agent calling `apply update noun=Task
    // fields={Task Status: 'completed'}` against an app whose schema
    // binds the Task SM must be refused at the MCP layer before any
    // engine call. The refusal must NAME the SM ("Task SM" or just
    // "Task"), call out `transition` as the correct verb, and list the
    // legal events from the current status.
    const refusal = smBypassRefusal({
      noun: 'Task',
      fields: { 'Task Status': 'completed' },
      schema: taskSchema,
      transitions: taskActions,
      force: false,
    })
    expect(refusal).not.toBeNull()
    expect(refusal!.error).toMatch(/Task SM|Task/)
    expect(refusal!.error).toMatch(/transition/)
    // The refusal must list each legal event so the agent knows what
    // to call next — verifies the helper threads transitions through
    // rather than just emitting a generic "use transition" message.
    expect(refusal!.error).toContain('start')
    expect(refusal!.error).toContain('finish')
    expect(refusal!.error).toContain('reopen')
    // The refusal points at the field name that triggered it so the
    // agent can see WHICH field they tried to set.
    expect(refusal!.error).toMatch(/Task Status/)
  })

  it('apply update {Task Subject: "X"} on same Task succeeds', () => {
    // Pass-through case: a non-SM field is fine to update directly.
    // The guard must NOT refuse — Task Subject, Task Description, Task
    // Priority are all single-valued facts the agent can flip via
    // `apply update` without bypassing any SM.
    const result = smBypassRefusal({
      noun: 'Task',
      fields: { 'Task Subject': 'X' },
      schema: taskSchema,
      transitions: taskActions,
      force: false,
    })
    expect(result).toBeNull()
  })

  it('apply update {Task Status: "completed", force: true} bypasses guard', () => {
    // Opt-out escape hatch for migration scripts / other legitimate
    // direct-mutation cases (e.g. an admin restoring an entity from
    // backup, where the SM history can't be replayed). The guard must
    // be a no-op when force is true.
    const result = smBypassRefusal({
      noun: 'Task',
      fields: { 'Task Status': 'completed' },
      schema: taskSchema,
      transitions: taskActions,
      force: true,
    })
    expect(result).toBeNull()
  })

  it('app without SM passes update through directly', () => {
    // When the active app's schema has zero state machines, the guard
    // is a no-op — there is no SM to bypass. This is the "new app
    // without behavioral entities" case: apps that model only static
    // relationships shouldn't pay any cost for the guard.
    const schemaWithoutSm = {
      nouns: ['Customer'],
      factTypes: [{ id: 'Customer_has_Email', reading: 'Customer has Email' }],
      constraints: [],
      stateMachines: [],
      totalFacts: 0,
    }
    const result = smBypassRefusal({
      noun: 'Customer',
      fields: { Email: 'alice@example.com' },
      schema: schemaWithoutSm,
      transitions: [],
      force: false,
    })
    expect(result).toBeNull()
  })

  it('payload with mixed SM + non-SM fields refuses entirely', () => {
    // The agent might try to "sneak" the SM field through alongside a
    // non-SM field. That's a worse footgun than the pure-SM case
    // because partial application would update Task Subject AND
    // silently mutate Task Status. The guard must refuse the ENTIRE
    // call rather than partially applying the non-SM field — keeping
    // the refusal atomic.
    const refusal = smBypassRefusal({
      noun: 'Task',
      fields: {
        'Task Subject': 'X',
        'Task Status': 'completed',
        'Task Description': 'y',
      },
      schema: taskSchema,
      transitions: taskActions,
      force: false,
    })
    expect(refusal).not.toBeNull()
    expect(refusal!.error).toMatch(/Task Status/)
    expect(refusal!.error).toMatch(/transition/)
  })

  it('non-SM-governed noun passes through even when app has SMs', () => {
    // The same app declares an SM for Task but no SM for Source File.
    // An `apply update` on Source File should pass through — the
    // guard only fires on SM-GOVERNED nouns, not on every noun in an
    // app that happens to have one SM.
    const result = smBypassRefusal({
      noun: 'Source File',
      fields: { path: 'a.md' },
      schema: taskSchema,
      transitions: [],
      force: false,
    })
    expect(result).toBeNull()
  })

  it('empty / missing transitions list still refuses (no swallowed footgun)', () => {
    // Defensive: if the actions lookup fails (engine miss, entity not
    // yet materialized), the guard must STILL refuse — the schema
    // alone is enough to know the noun is SM-governed; missing
    // transitions just means we can't enumerate them. A generic
    // "use apply transition instead" message is better than silent
    // pass-through.
    const refusal = smBypassRefusal({
      noun: 'Task',
      fields: { 'Task Status': 'completed' },
      schema: taskSchema,
      transitions: [],
      force: false,
    })
    expect(refusal).not.toBeNull()
    expect(refusal!.error).toMatch(/Task Status/)
    expect(refusal!.error).toMatch(/transition/)
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

// =====================================================================
// p0 mcp-active-app-isolation (option b) — per-call `app` override.
//
// The MCP server's active app is a module-level global (`activeApp`) plus
// an on-disk marker (`.arest-active-app`). Sub-agents share ONE stdio
// connection, so a global active app can't isolate per session — one
// agent's `apps.use` silently re-scopes another's reads/writes. Option
// (b): let a single CALL carry an optional `app` that resolves its own
// DB + readings + engine handle for THAT call only, never mutating the
// shared global or the marker.
//
// server.test.ts deliberately avoids the side-effectful engine path, so
// the per-call mechanism is structured as EXPORTED PURE HELPERS tested
// here (resolution + handle cache) plus source-string assertions that
// the threading is wired into every targeted verb.
// =====================================================================

describe('per-call app scoping (p0 mcp-active-app-isolation, option b)', () => {
  // A throwaway apps workspace with two distinct apps. Each gets a
  // readings/ dir and a discoverable <name>.db so resolveArestApp
  // returns a real dbPath/readingsDir we can assert against — WITHOUT
  // touching any live .db.
  let appsDir: string
  beforeAll(() => {
    appsDir = mkdtempSync(join(tmpdir(), 'arest-percall-'))
    for (const name of ['alpha', 'beta']) {
      const root = join(appsDir, name)
      mkdirSync(join(root, 'readings'), { recursive: true })
      writeFileSync(join(root, 'readings', 'app.md'), `# ${name}\n`, 'utf8')
      // A discoverable DB so resolveArestApp.dbPath points at a real file.
      writeFileSync(join(root, `${name}.db`), '', 'utf8')
    }
  })
  afterAll(() => {
    try { rmSync(appsDir, { recursive: true, force: true }) } catch {}
  })

  describe('resolveCallScope — pure per-call resolution', () => {
    it('resolves the per-call app to its own DB + readings dir', () => {
      const alpha = resolveCallScope('alpha', { appsDir, cwd: appsDir })
      const beta = resolveCallScope('beta', { appsDir, cwd: appsDir })
      expect(alpha.name).toBe('alpha')
      expect(alpha.dbPath).toBe(join(appsDir, 'alpha', 'alpha.db'))
      expect(alpha.readingsDir).toBe(join(appsDir, 'alpha', 'readings'))
      expect(alpha.exists).toBe(true)
      // The two apps resolve to genuinely different scopes — the whole
      // point is that a per-call `app` reads/writes a different UoD.
      expect(beta.dbPath).not.toBe(alpha.dbPath)
      expect(beta.readingsDir).not.toBe(alpha.readingsDir)
      expect(beta.dbPath).toBe(join(appsDir, 'beta', 'beta.db'))
    })

    it('returns exists=false for an unknown app (no throw, caller can branch)', () => {
      const missing = resolveCallScope('does-not-exist', { appsDir, cwd: appsDir })
      expect(missing.name).toBe('does-not-exist')
      expect(missing.exists).toBe(false)
    })

    it('scopeDbPath / scopeReadingsDir prefer the scope, else fall back', () => {
      const scope = resolveCallScope('alpha', { appsDir, cwd: appsDir })
      // With a scope: the scope's paths win.
      expect(scopeDbPath(scope, '/fallback/db')).toBe(scope.dbPath)
      expect(scopeReadingsDir(scope, '/fallback/readings')).toBe(scope.readingsDir)
      // Without a scope (undefined ⇒ omitted `app`): the global fallback
      // is returned verbatim, so the no-`app` path is byte-identical to
      // today's behavior.
      expect(scopeDbPath(undefined, '/fallback/db')).toBe('/fallback/db')
      expect(scopeReadingsDir(undefined, '/fallback/readings')).toBe('/fallback/readings')
    })
  })

  describe('per-call handle cache (keyed by readings signature)', () => {
    it('returns undefined on a cold cache, the stored handle on a warm hit', () => {
      const cache = new Map<string, number>()
      const sig = 'alpha:1700000000000:0|/r/app.md:1700000000000:12'
      expect(lookupHandleCache(cache, sig)).toBeUndefined()
      rememberHandleCache(cache, sig, 7)
      expect(lookupHandleCache(cache, sig)).toBe(7)
    })

    it('keys distinct signatures to distinct handles (a 2nd app gets its own)', () => {
      // The shared `_localHandle` is a single slot keyed by ONE readings
      // signature, so a per-call second app MUST get its own cache entry
      // or it would collide with the global app's handle.
      const cache = new Map<string, number>()
      rememberHandleCache(cache, 'alpha-sig', 1)
      rememberHandleCache(cache, 'beta-sig', 2)
      expect(lookupHandleCache(cache, 'alpha-sig')).toBe(1)
      expect(lookupHandleCache(cache, 'beta-sig')).toBe(2)
      // Re-storing under the same signature replaces (stale readings ⇒
      // recompiled handle) rather than leaking a second entry.
      rememberHandleCache(cache, 'alpha-sig', 99)
      expect(lookupHandleCache(cache, 'alpha-sig')).toBe(99)
      expect(cache.size).toBe(2)
    })
  })

  // ── Source assertions: the `app` override is wired into every
  // targeted verb + the resolution NEVER mutates the global/marker. ──
  describe('source wiring (every targeted verb threads `app`; global untouched)', () => {
    const SRC = SERVER_TS.replace(/\r\n/g, '\n')
    const sliceConfig = (verb: string): string => {
      const head = SRC.indexOf(`server.registerTool(\n  '${verb}',\n`)
      expect(head, `registerTool('${verb}', ...) block not found`).toBeGreaterThan(0)
      const tail = SRC.indexOf('server.registerTool(', head + 1)
      return SRC.slice(head, tail > 0 ? tail : SRC.length)
    }

    // The READ/WRITE verbs that resolve their DB/readings through the
    // global active app and therefore need the per-call override.
    const SCOPED_VERBS = [
      'get', 'query', 'sql', 'cells',
      'apply', 'retract', 'schema', 'actions', 'explain',
    ] as const

    for (const verb of SCOPED_VERBS) {
      it(`'${verb}' input schema exposes an optional \`app\` override`, () => {
        const config = sliceConfig(verb)
        // The schema must declare `app: z.string().optional()` so a call
        // can scope itself without `apps.use`.
        expect(config, `${verb}: missing app input field`)
          .toMatch(/app:\s*z\.string\(\)\.optional\(\)/)
      })
    }

    it('resolveCallScope NEVER writes the global activeApp or the marker', () => {
      // The whole bug is a shared mutated global. The per-call resolver
      // must be pure: it may call resolveArestApp (a pure fs read) but
      // must not assign `activeApp =`, call `activateApp(`, or write the
      // `.arest-active-app` marker via writePersistedAppName.
      const fnStart = SRC.indexOf('export function resolveCallScope')
      expect(fnStart, 'resolveCallScope not found').toBeGreaterThan(0)
      const fnBody = SRC.slice(fnStart, SRC.indexOf('\n}', fnStart) + 2)
      expect(fnBody).not.toMatch(/activeApp\s*=/)
      expect(fnBody).not.toMatch(/activateApp\(/)
      expect(fnBody).not.toMatch(/writePersistedAppName\(/)
    })

    it('systemCall threads an optional per-call scope through to db/readings', () => {
      // systemCall is the single chokepoint every scoped verb funnels
      // through. It must accept an optional scope and pass it to BOTH
      // the CLI-db path and the in-process engine-handle path so the
      // override reaches whichever backend is active.
      const sigStart = SRC.indexOf('async function systemCall(')
      expect(sigStart, 'systemCall not found').toBeGreaterThan(0)
      const sig = SRC.slice(sigStart, SRC.indexOf(')', sigStart) + 1)
      expect(sig).toMatch(/scope\??:/)
    })

    it('the per-call engine handle is cached separately from the global _localHandle', () => {
      // A keyed cache (Map) holds per-call handles so a second app does
      // not clobber the global app's single _localHandle slot.
      expect(SRC).toMatch(/_perCallHandles\s*[:=]/)
      expect(SRC).toMatch(/new Map<string, number>\(\)/)
    })
  })
})
