/**
 * GraphDL MCP Server — stdio transport.
 *
 * Exposes the AREST engine as MCP tools so an AI agent (Claude Desktop,
 * Claude Code, etc.) can list/create/query entities, compile readings,
 * inspect audit trails, and verify identity signatures.
 *
 * Two modes (selected by env):
 *   GRAPHDL_MODE=local     — load readings from $GRAPHDL_READINGS_DIR via
 *                            the bundled WASM engine. No network. Default
 *                            when GRAPHDL_URL is unset or empty.
 *   GRAPHDL_MODE=remote    — call a deployed Cloudflare Worker at
 *                            $GRAPHDL_URL using $GRAPHDL_API_KEY.
 *
 * Usage from a plugin config (Claude Desktop / Claude Code):
 *   {
 *     "mcpServers": {
 *       "graphdl": {
 *         "command": "npx",
 *         "args": ["-y", "graphdl-orm", "mcp"],
 *         "env": {
 *           "GRAPHDL_MODE": "local",
 *           "GRAPHDL_READINGS_DIR": "/absolute/path/to/readings"
 *         }
 *       }
 *     }
 *   }
 *
 * Or call directly:
 *   GRAPHDL_MODE=local GRAPHDL_READINGS_DIR=./readings npx tsx src/mcp/server.ts
 */

/// <reference types="node" />
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { z } from 'zod'
import { readFileSync, readdirSync, existsSync } from 'fs'
import { resolve, dirname, join } from 'path'
import { fileURLToPath } from 'url'

const __dirname = dirname(fileURLToPath(import.meta.url))

// ── Mode selection ──────────────────────────────────────────────────

const GRAPHDL_URL = process.env.GRAPHDL_URL || ''
const GRAPHDL_API_KEY = process.env.GRAPHDL_API_KEY || ''
const GRAPHDL_READINGS_DIR = process.env.GRAPHDL_READINGS_DIR || ''
const GRAPHDL_MODE = (process.env.GRAPHDL_MODE || (GRAPHDL_URL ? 'remote' : 'local')).toLowerCase()
const GRAPHDL_DEBUG = process.env.GRAPHDL_DEBUG === '1'

// ── Local mode: bundled WASM engine via engine.ts ───────────────────
// Lazily imported so remote-mode users don't pay the WASM cost.

let _localHandle: number = -1
let _localEngine: typeof import('../api/engine.js') | null = null

async function getLocalEngine() {
  if (_localEngine) return _localEngine
  _localEngine = await import('../api/engine.js')
  return _localEngine
}

async function getLocalHandle(): Promise<number> {
  if (_localHandle >= 0) return _localHandle
  const engine = await getLocalEngine()
  const readings = loadReadingsFromDir(GRAPHDL_READINGS_DIR)
  _localHandle = engine.compileDomainReadings(...readings)
  return _localHandle
}

function loadReadingsFromDir(dir: string): string[] {
  if (!dir || !existsSync(dir)) return []
  return readdirSync(dir)
    .filter(name => name.endsWith('.md'))
    .sort()
    .map(name => readFileSync(join(dir, name), 'utf-8'))
}

// ── Remote mode: HTTP fetch ─────────────────────────────────────────

async function httpRequest(path: string, options?: RequestInit): Promise<any> {
  const url = `${GRAPHDL_URL}${path}`
  const headers: Record<string, string> = {
    'Accept': 'application/json',
    'Content-Type': 'application/json',
  }
  if (GRAPHDL_API_KEY) {
    headers['Authorization'] = `Bearer ${GRAPHDL_API_KEY}`
  }
  const res = await fetch(url, { ...options, headers: { ...headers, ...options?.headers } })
  if (!res.ok) {
    const text = await res.text().catch(() => '')
    throw new Error(`${res.status} ${res.statusText}: ${text}`)
  }
  return res.json()
}

function textResult(data: any) {
  return { content: [{ type: 'text' as const, text: JSON.stringify(data, null, 2) }] }
}

// ── Command dispatch (dual mode) ────────────────────────────────────

async function dispatchCommand(command: any): Promise<any> {
  if (GRAPHDL_MODE === 'local') {
    const engine = await getLocalEngine()
    const handle = await getLocalHandle()
    const raw = engine.system(handle, 'apply', JSON.stringify(command))
    try { return JSON.parse(raw) } catch { return { rejected: true, error: raw } }
  }
  // Remote: POST to /arest/:domain/:noun or /arest/:domain/apply
  return httpRequest(`/arest/${command.domain || 'default'}/apply`, {
    method: 'POST',
    body: JSON.stringify(command),
  })
}

async function dispatchRead(path: string): Promise<any> {
  if (GRAPHDL_MODE === 'local') {
    const engine = await getLocalEngine()
    const handle = await getLocalHandle()
    const raw = engine.system(handle, 'debug', '')
    try { return JSON.parse(raw) } catch { return { raw } }
  }
  return httpRequest(path)
}

const server = new McpServer({
  name: 'graphdl',
  version: '0.1.0',
})

// ── Read: list entities of a noun type ──────────────────────────────

server.tool(
  'graphdl_list',
  'List entities of a noun type in a domain',
  {
    noun: z.string().describe('The noun type (e.g. "Order", "Customer")'),
    domain: z.string().describe('The domain slug (e.g. "support", "core")'),
    page: z.number().optional().describe('Page number (default 1)'),
    limit: z.number().optional().describe('Items per page (default 100)'),
  },
  async ({ noun, domain, page, limit }) => {
    if (GRAPHDL_MODE === 'local') {
      const engine = await getLocalEngine()
      const handle = await getLocalHandle()
      const raw = engine.system(handle, `list:${noun}`, domain)
      try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
    }
    const params = new URLSearchParams()
    if (page) params.set('page', String(page))
    if (limit) params.set('limit', String(limit))
    const qs = params.toString()
    const data = await httpRequest(`/arest/${domain}/${encodeURIComponent(noun)}${qs ? '?' + qs : ''}`)
    return textResult(data)
  },
)

// ── Read Detail: get a specific entity ──────────────────────────────

server.tool(
  'graphdl_get',
  'Get a specific entity by ID',
  {
    noun: z.string().describe('The noun type'),
    domain: z.string().describe('The domain slug'),
    id: z.string().describe('The entity ID'),
  },
  async ({ noun, domain, id }) => {
    if (GRAPHDL_MODE === 'local') {
      const engine = await getLocalEngine()
      const handle = await getLocalHandle()
      const raw = engine.system(handle, `get:${noun}`, id)
      try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
    }
    const data = await httpRequest(`/arest/${domain}/${encodeURIComponent(noun)}/${encodeURIComponent(id)}`)
    return textResult(data)
  },
)

// ── Create: create a new entity (AREST create = emit ∘ validate ∘ derive ∘ resolve)

server.tool(
  'graphdl_create',
  'Create a new entity. Executes the AREST create pipeline (resolve → derive → validate → emit). Returns the created entity with HATEOAS links, or a violation set on rejection. Identity (sender) is pushed as a User fact during resolve; omit to skip identity enforcement.',
  {
    noun: z.string().describe('The noun type to create'),
    domain: z.string().describe('The domain slug'),
    id: z.string().optional().describe('Explicit entity id. If omitted the engine generates one.'),
    fields: z.record(z.string(), z.string()).describe('Field values keyed by role name'),
    sender: z.string().optional().describe('Caller identity (typically an email). Pushed as User fact during resolve for constraint-based authorization.'),
    signature: z.string().optional().describe('Hash-based MAC over sender+payload. Verified via crypto::verify_signature if present.'),
  },
  async ({ noun, domain, id, fields, sender, signature }) => {
    const command = { type: 'createEntity', noun, domain, id, fields, sender, signature }
    const data = await dispatchCommand(command)
    return textResult(data)
  },
)

// ── Apply: generic Command dispatch ──────────────────────────────────

server.tool(
  'graphdl_apply',
  'Generic command dispatch. One tool exposes all 5 AREST Command variants (createEntity, transition, query, updateEntity, loadReadings). Prefer the specific tools (graphdl_create, graphdl_transition, etc.) when possible.',
  {
    command: z.record(z.string(), z.any()).describe('Command object with a "type" field and variant-specific fields. See the Command enum in crates/arest/src/arest.rs for exact shape.'),
  },
  async ({ command }) => {
    const data = await dispatchCommand(command)
    return textResult(data)
  },
)

// ── Transition: state machine event dispatch ────────────────────────

server.tool(
  'graphdl_transition',
  'Fire a state machine transition event on an entity. Backus foldl transition over the event stream.',
  {
    entityId: z.string().describe('Target entity id'),
    event: z.string().describe('Transition event name (e.g. "place", "ship")'),
    domain: z.string().describe('Domain slug'),
    currentStatus: z.string().optional().describe('Current status (optional; engine resolves from state if omitted)'),
    sender: z.string().optional(),
    signature: z.string().optional(),
  },
  async ({ entityId, event, domain, currentStatus, sender, signature }) => {
    const command = { type: 'transition', entityId, event, domain, currentStatus, sender, signature }
    const data = await dispatchCommand(command)
    return textResult(data)
  },
)

// ── Evaluate: run constraint evaluation ─────────────────────────────

server.tool(
  'graphdl_evaluate',
  'Evaluate constraints against a response. Returns the violation set.',
  {
    domain: z.string().describe('The domain slug'),
    response: z.record(z.string(), z.any()).describe('The response data to evaluate'),
  },
  async ({ domain, response }) => {
    if (GRAPHDL_MODE === 'local') {
      const engine = await getLocalEngine()
      const handle = await getLocalHandle()
      const raw = engine.system(handle, 'evaluate', JSON.stringify({ domain, response }))
      try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
    }
    const data = await httpRequest(`/arest/${domain}/evaluate`, {
      method: 'POST',
      body: JSON.stringify({ response }),
    })
    return textResult(data)
  },
)

// ── Schema: get noun/fact/constraint schemas for a domain ───────────

server.tool(
  'graphdl_schema',
  'Get the schema (nouns, fact types, constraints, state machines) for a domain',
  {
    domain: z.string().describe('The domain slug'),
  },
  async ({ domain }) => {
    if (GRAPHDL_MODE === 'local') {
      const data = await dispatchRead('/schema')
      return textResult(data)
    }
    const data = await httpRequest(`/arest/${domain}/schema`)
    return textResult(data)
  },
)

// ── Compile: ingest new FORML2 readings (self-modification) ─────────

server.tool(
  'graphdl_compile',
  'Compile FORML2 readings into the domain. This is AREST self-modification (Corollary 2, Closure Under Self-Modification): the engine extends its own program. Alethic constraint violations in the merged state will reject the compile.',
  {
    domain: z.string().describe('The domain slug'),
    readings: z.string().describe('FORML2 readings as markdown text'),
  },
  async ({ domain, readings }) => {
    if (GRAPHDL_MODE === 'local') {
      const engine = await getLocalEngine()
      const handle = await getLocalHandle()
      const raw = engine.system(handle, 'compile', readings)
      return textResult({ ok: !raw.startsWith('⊥'), result: raw })
    }
    const data = await httpRequest('/parse', {
      method: 'POST',
      body: JSON.stringify({ domain, text: readings }),
    })
    return textResult(data)
  },
)

// Back-compat alias for graphdl_parse
server.tool(
  'graphdl_parse',
  'Alias for graphdl_compile. Prefer graphdl_compile in new code.',
  {
    domain: z.string().describe('The domain slug'),
    readings: z.string().describe('FORML2 readings as markdown text'),
  },
  async ({ domain, readings }) => {
    if (GRAPHDL_MODE === 'local') {
      const engine = await getLocalEngine()
      const handle = await getLocalHandle()
      const raw = engine.system(handle, 'compile', readings)
      return textResult({ ok: !raw.startsWith('⊥'), result: raw })
    }
    const data = await httpRequest('/parse', {
      method: 'POST',
      body: JSON.stringify({ domain, text: readings }),
    })
    return textResult(data)
  },
)

// ── Audit log: read the compile/apply audit trail (#26) ─────────────

server.tool(
  'graphdl_audit_log',
  'Read the audit_log cell: (operation, outcome, sequence, sender) for every compile and apply operation. Monotonic sequence per cell. Local mode only — remote mode requires the worker to expose /audit.',
  {
    limit: z.number().optional().describe('Max entries to return (default: all)'),
  },
  async ({ limit }) => {
    if (GRAPHDL_MODE !== 'local') {
      return textResult({ error: 'audit_log is only available in local mode' })
    }
    const engine = await getLocalEngine()
    const handle = await getLocalHandle()
    const raw = engine.system(handle, 'audit_log', String(limit ?? 0))
    try {
      const data = JSON.parse(raw)
      return textResult(data)
    } catch {
      return textResult({ raw })
    }
  },
)

// ── Verify signature (#24) ──────────────────────────────────────────

server.tool(
  'graphdl_verify_signature',
  'Verify a sender/payload/signature tuple against the engine\'s crypto primitive. Returns boolean. Used by anonymous peers per AREST §5.5.',
  {
    sender: z.string().describe('Claimed sender identity'),
    payload: z.string().describe('Signed payload'),
    signature: z.string().describe('Signature to verify'),
  },
  async ({ sender, payload, signature }) => {
    if (GRAPHDL_MODE === 'local') {
      const engine = await getLocalEngine()
      const handle = await getLocalHandle()
      const encoded = `<${sender},${payload},${signature}>`
      const raw = engine.system(handle, 'verify_signature', encoded)
      return textResult({ valid: raw === 'true', raw })
    }
    const data = await httpRequest('/crypto/verify', {
      method: 'POST',
      body: JSON.stringify({ sender, payload, signature }),
    })
    return textResult(data)
  },
)

// ── Debug: state projection (gated by GRAPHDL_DEBUG=1, #18) ─────────

if (GRAPHDL_DEBUG) {
  server.tool(
    'graphdl_debug',
    'Dump the full compiled state (nouns, fact types, constraints, state machines). Development only — enable with GRAPHDL_DEBUG=1.',
    {},
    async () => {
      if (GRAPHDL_MODE === 'local') {
        const engine = await getLocalEngine()
        const handle = await getLocalHandle()
        const raw = engine.system(handle, 'debug', '')
        try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
      }
      const data = await httpRequest('/debug')
      return textResult(data)
    },
  )
}

// ── Prompts — domain knowledge served on demand ─────────────────────

function loadPrompt(name: string): string {
  try {
    return readFileSync(resolve(__dirname, 'prompts', `${name}.md`), 'utf-8')
  } catch {
    return `# ${name}\n\nPrompt file not found.`
  }
}

server.prompt(
  'graphdl_overview',
  'GraphDL system overview, constraint types, and FORML2 document structure',
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('overview') } }] }),
)

server.prompt(
  'graphdl_modeling',
  'Domain modeling guide: entity/value types, multiplicity, subtypes, derivation rules',
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('modeling') } }] }),
)

server.prompt(
  'graphdl_principles',
  'Design principles: encapsulation, no bridge architecture, no IR, the paper is the spec',
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('design-principles') } }] }),
)

// ── Start ───────────────────────────────────────────────────────────

async function main() {
  const transport = new StdioServerTransport()
  await server.connect(transport)
  // eslint-disable-next-line no-console
  console.error(`GraphDL MCP server started — mode=${GRAPHDL_MODE}${GRAPHDL_MODE === 'remote' ? ` url=${GRAPHDL_URL}` : ''}${GRAPHDL_DEBUG ? ' [DEBUG]' : ''}`)
}

main().catch((err) => {
  console.error('GraphDL MCP server failed:', err)
  process.exit(1)
})
