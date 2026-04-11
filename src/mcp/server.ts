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

server.registerTool(
  'graphdl_list',
  {
    description: 'List entities of a noun type in a domain',
    inputSchema: {
      noun: z.string().describe('The noun type (e.g. "Order", "Customer")'),
      domain: z.string().describe('The domain slug (e.g. "support", "core")'),
      page: z.number().optional().describe('Page number (default 1)'),
      limit: z.number().optional().describe('Items per page (default 100)'),
    },
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

server.registerTool(
  'graphdl_get',
  {
    description: 'Get a specific entity by ID',
    inputSchema: {
      noun: z.string().describe('The noun type'),
      domain: z.string().describe('The domain slug'),
      id: z.string().describe('The entity ID'),
    },
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

server.registerTool(
  'graphdl_create',
  {
    description: 'Create a new entity. Executes the AREST create pipeline (resolve → derive → validate → emit). Returns the created entity with HATEOAS links, or a violation set on rejection.',
    inputSchema: {
      noun: z.string().describe('The noun type to create'),
      domain: z.string().describe('The domain slug'),
      id: z.string().optional().describe('Explicit entity id. If omitted the engine generates one.'),
      fields: z.record(z.string(), z.string()).describe('Field values keyed by role name'),
      sender: z.string().optional().describe('Caller identity (typically an email). Pushed as User fact during resolve.'),
      signature: z.string().optional().describe('HMAC-SHA256 over sender+payload.'),
    },
  },
  async ({ noun, domain, id, fields, sender, signature }) => {
    const command = { type: 'createEntity', noun, domain, id, fields, sender, signature }
    const data = await dispatchCommand(command)
    return textResult(data)
  },
)

// ── Apply: generic Command dispatch ──────────────────────────────────

server.registerTool(
  'graphdl_apply',
  {
    description: 'Generic command dispatch. Exposes all 5 AREST Command variants. Prefer specific tools when possible.',
    inputSchema: {
      command: z.record(z.string(), z.any()).describe('Command object with a "type" field and variant-specific fields.'),
    },
  },
  async ({ command }) => {
    const data = await dispatchCommand(command)
    return textResult(data)
  },
)

// ── Transition: state machine event dispatch ────────────────────────

server.registerTool(
  'graphdl_transition',
  {
    description: 'Fire a state machine transition on an entity.',
    inputSchema: {
      entityId: z.string().describe('Target entity id'),
      event: z.string().describe('Transition event name (e.g. "place", "ship")'),
      domain: z.string().describe('Domain slug'),
      currentStatus: z.string().optional().describe('Current status (optional; engine resolves from state if omitted)'),
      sender: z.string().optional(),
      signature: z.string().optional(),
    },
  },
  async ({ entityId, event, domain, currentStatus, sender, signature }) => {
    const command = { type: 'transition', entityId, event, domain, currentStatus, sender, signature }
    const data = await dispatchCommand(command)
    return textResult(data)
  },
)

// ── Evaluate: run constraint evaluation ─────────────────────────────

server.registerTool(
  'graphdl_evaluate',
  {
    description: 'Evaluate constraints against a response. Returns the violation set.',
    inputSchema: {
      domain: z.string().describe('The domain slug'),
      response: z.record(z.string(), z.any()).describe('The response data to evaluate'),
    },
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

server.registerTool(
  'graphdl_schema',
  {
    description: 'Get the schema (nouns, fact types, constraints, state machines) for a domain',
    inputSchema: {
      domain: z.string().describe('The domain slug'),
    },
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

server.registerTool(
  'graphdl_compile',
  {
    description: 'Compile FORML2 readings (self-modification, Corollary 5). Alethic violations reject.',
    inputSchema: {
      domain: z.string().describe('The domain slug'),
      readings: z.string().describe('FORML2 readings as markdown text'),
    },
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
server.registerTool(
  'graphdl_parse',
  {
    description: 'Alias for graphdl_compile. Prefer graphdl_compile.',
    inputSchema: {
      domain: z.string().describe('The domain slug'),
      readings: z.string().describe('FORML2 readings as markdown text'),
    },
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

server.registerTool(
  'graphdl_audit_log',
  {
    description: 'Read the audit_log cell. Monotonic sequence per cell. Local mode only.',
    inputSchema: {
      limit: z.number().optional().describe('Max entries to return (default: all)'),
    },
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

server.registerTool(
  'graphdl_verify_signature',
  {
    description: 'Verify HMAC-SHA256 sender/payload/signature tuple.',
    inputSchema: {
      sender: z.string().describe('Claimed sender identity'),
      payload: z.string().describe('Signed payload'),
      signature: z.string().describe('Signature to verify'),
    },
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
  server.registerTool(
    'graphdl_debug',
    {
      description: 'Dump the full compiled state. Development only — enable with GRAPHDL_DEBUG=1.',
    },
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

server.registerPrompt(
  'graphdl_overview',
  { description: 'GraphDL system overview, constraint types, and FORML2 document structure' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('overview') } }] }),
)

server.registerPrompt(
  'graphdl_entity_modeling',
  { description: 'Entity/value types, reference schemes, normalization, arity, multiplicity, objectification' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('entity-modeling') } }] }),
)

server.registerPrompt(
  'graphdl_advanced_constraints',
  { description: 'Subtype partitions, subset constraints with autofill, ring constraints' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('advanced-constraints') } }] }),
)

server.registerPrompt(
  'graphdl_derivation_deontic',
  { description: 'Derivation rules, deontic vs alethic modality, obligatory/forbidden/permitted operators' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('derivation-deontic') } }] }),
)

server.registerPrompt(
  'graphdl_verbalization',
  { description: 'Full ORM2 verbalization tables: UC, MC, DMaC, SSC, combined patterns from Halpin ORM2-02' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('verbalization') } }] }),
)

server.registerPrompt(
  'graphdl_principles',
  { description: 'Design principles: facts all the way down, no bridge architecture, the paper is the spec' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('design-principles') } }] }),
)

server.registerPrompt(
  'graphdl_api',
  { description: 'AREST API reference: CLI keys, MCP tools, HTTP endpoints, identity/signing' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('api') } }] }),
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
