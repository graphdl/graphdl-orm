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

// ── Local system call helper ──────────────────────────────────────

async function systemCall(key: string, input: string): Promise<string> {
  const engine = await getLocalEngine()
  const handle = await getLocalHandle()
  return engine.system(handle, key, input)
}

// ── Data Federation: fetch from external systems via populate:{noun} ──

interface FederationConfig {
  system: string
  url: string
  uri: string
  header: string
  prefix: string
  noun: string
  fields: string[]
}

/** Parse a populate:{noun} def from the engine into a FederationConfig. */
function parseFederationConfig(raw: string): FederationConfig | null {
  try {
    // The def is an Object sequence of pairs: <<key, value>, ...>
    // Parse the simple pattern: <key, value> pairs
    const config: Record<string, string | string[]> = {}
    const pairRe = /<([^,<>]+),\s*([^<>]*?)>/g
    let match
    while ((match = pairRe.exec(raw)) !== null) {
      const [, key, value] = match
      config[key.trim()] = value.trim()
    }
    // Fields is a nested sequence — extract from the raw string
    const fieldsMatch = raw.match(/fields,\s*<([^>]*)>/)
    const fields = fieldsMatch
      ? fieldsMatch[1].split(',').map(s => s.trim().replace(/^'|'$/g, ''))
      : []
    return {
      system: String(config['system'] || ''),
      url: String(config['url'] || ''),
      uri: String(config['uri'] || ''),
      header: String(config['header'] || ''),
      prefix: String(config['prefix'] || ''),
      noun: String(config['noun'] || ''),
      fields,
    }
  } catch {
    return null
  }
}

/** Fetch facts from an external system using a populate config. */
async function federatedFetch(config: FederationConfig, entityId?: string): Promise<any> {
  const baseUrl = config.url.replace(/\/$/, '')
  const path = config.uri.replace(/^\//, '')
  const url = entityId
    ? `${baseUrl}/${path}/${encodeURIComponent(entityId)}`
    : `${baseUrl}/${path}`

  const headers: Record<string, string> = {
    'Accept': 'application/json',
  }

  // Inject auth from env: AREST_SECRET_{SYSTEM_NAME} (uppercase, underscored)
  const envKey = `AREST_SECRET_${config.system.replace(/[^a-zA-Z0-9]/g, '_').toUpperCase()}`
  const secret = process.env[envKey] || ''
  if (config.header && secret) {
    const value = config.prefix ? `${config.prefix} ${secret}` : secret
    headers[config.header] = value
  }

  const res = await fetch(url, { headers })
  if (!res.ok) {
    return { error: `${res.status} ${res.statusText}`, url, system: config.system }
  }
  const json = await res.json() as any

  // Map JSON response to facts using field names from the config.
  // The response is either an object (single entity) or array (list).
  const items = Array.isArray(json.data) ? json.data : Array.isArray(json) ? json : [json]
  return {
    system: config.system,
    noun: config.noun,
    count: items.length,
    facts: items.map((item: any) => {
      const bindings: Record<string, string> = {}
      // Map each field to a role binding
      config.fields.forEach(field => {
        const snakeField = field.toLowerCase().replace(/ /g, '_')
        // Try exact match, then snake_case, then camelCase
        const val = item[field] ?? item[snakeField] ?? item[field.replace(/ /g, '')]
        if (val !== undefined) bindings[field] = String(val)
      })
      // Include the entity ID
      if (item.id) bindings[config.noun] = String(item.id)
      return bindings
    }),
    _meta: { url, worldAssumption: 'OWA' },
  }
}

/** Check if a noun has a populate def and return its config. */
async function getFederationConfig(noun: string): Promise<FederationConfig | null> {
  if (GRAPHDL_MODE !== 'local') return null
  try {
    const raw = await systemCall(`populate:${noun}`, '')
    if (raw.startsWith('⊥') || raw === 'φ') return null
    return parseFederationConfig(raw)
  } catch {
    return null
  }
}

const server = new McpServer({
  name: 'graphdl',
  version: '0.2.0',
})

// =====================================================================
// TOOLS — 6 core verbs for agent ergonomics
// =====================================================================

// ── 1. get: retrieve an entity or list entities ──────────────────────

server.registerTool(
  'get',
  {
    description: 'Get an entity by ID, or list all entities of a noun type. Returns the entity with its current state, HATEOAS links, and navigation.',
    inputSchema: {
      id: z.string().optional().describe('Entity ID. If omitted, lists all entities of the noun type.'),
      noun: z.string().optional().describe('Noun type (e.g. "Order"). Required when listing, optional when getting by ID (inferred from population).'),
    },
  },
  async ({ id, noun }) => {
    if (!noun) return textResult({ error: 'Provide noun to get or list.' })

    // Check if this noun is backed by an external system (data federation).
    const fedConfig = await getFederationConfig(noun)
    if (fedConfig) {
      const data = await federatedFetch(fedConfig, id || undefined)
      return textResult(data)
    }

    // Local population
    if (GRAPHDL_MODE === 'local') {
      if (id) {
        const raw = await systemCall(`get:${noun}`, id)
        try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
      }
      const raw = await systemCall(`list:${noun}`, '')
      try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
    }
    const path = id
      ? `/arest/default/${encodeURIComponent(noun)}/${encodeURIComponent(id)}`
      : `/arest/default/${encodeURIComponent(noun)}`
    const data = await httpRequest(path)
    return textResult(data)
  },
)

// ── 2. query: query facts across the population ──────────────────────

server.registerTool(
  'query',
  {
    description: 'Query facts by fact type. Returns matching facts from the population. Use to explore relationships between entities.',
    inputSchema: {
      fact_type: z.string().describe('Fact type ID (e.g. "Order_was_placed_by_Customer", "Case_has_Observation")'),
      filter: z.record(z.string(), z.string()).optional().describe('Filter by role bindings (e.g. {"Case": "The Speckled Band"})'),
    },
  },
  async ({ fact_type, filter }) => {
    if (GRAPHDL_MODE === 'local') {
      const filterStr = filter ? JSON.stringify(filter) : ''
      const raw = await systemCall(`query:${fact_type}`, filterStr)
      try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
    }
    const data = await httpRequest(`/arest/default/query/${encodeURIComponent(fact_type)}`, {
      method: 'POST',
      body: JSON.stringify({ filter }),
    })
    return textResult(data)
  },
)

// ── 3. apply: create, update, or transition an entity ────────────────

server.registerTool(
  'apply',
  {
    description: 'Apply an operation to an entity. The operation determines behavior: create (new entity), update (modify fields), transition (fire SM event). Executes the AREST pipeline: resolve → derive → validate → emit.',
    inputSchema: {
      operation: z.enum(['create', 'update', 'transition']).describe('Operation type'),
      noun: z.string().describe('Entity noun type (e.g. "Order", "Case")'),
      id: z.string().optional().describe('Entity ID. Required for update/transition. Optional for create (auto-generated).'),
      fields: z.record(z.string(), z.string()).optional().describe('Fact pairs for create/update (e.g. {"Name": "Acme", "customer": "alice"})'),
      event: z.string().optional().describe('SM event for transition (e.g. "place", "ship")'),
      sender: z.string().optional().describe('Caller identity for authorization'),
      signature: z.string().optional().describe('HMAC-SHA256 signature'),
    },
  },
  async ({ operation, noun, id, fields, event, sender, signature }) => {
    if (GRAPHDL_MODE === 'local') {
      switch (operation) {
        case 'create': {
          const pairs = Object.entries(fields || {}).map(([k, v]) => `<${k}, ${v}>`).join(', ')
          const idPair = id ? `<id, ${id}>, ` : ''
          const raw = await systemCall(`create:${noun}`, `<${idPair}${pairs}>`)
          try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
        }
        case 'update': {
          const command = { type: 'updateEntity', noun, domain: '', entityId: id, fields: fields || {}, sender, signature }
          const data = await dispatchCommand(command)
          return textResult(data)
        }
        case 'transition': {
          const command = { type: 'transition', entityId: id, event, domain: '', sender, signature }
          const data = await dispatchCommand(command)
          return textResult(data)
        }
      }
    }
    // Remote mode: dispatch via HTTP
    const command = operation === 'create'
      ? { type: 'createEntity', noun, domain: '', id, fields, sender, signature }
      : operation === 'update'
        ? { type: 'updateEntity', noun, domain: '', entityId: id, fields, sender, signature }
        : { type: 'transition', entityId: id, event, domain: '', sender, signature }
    const data = await httpRequest('/arest/default/apply', { method: 'POST', body: JSON.stringify(command) })
    return textResult(data)
  },
)

// ── 4. actions: get valid actions for an entity (HATEOAS) ────────────

server.registerTool(
  'actions',
  {
    description: 'Get valid actions for an entity. Returns available SM transitions, navigation links (parent/child), and applicable operations. Pure HATEOAS — the agent discovers what is possible without knowing the schema.',
    inputSchema: {
      noun: z.string().describe('Entity noun type'),
      id: z.string().describe('Entity ID'),
      status: z.string().optional().describe('Current SM status (resolved from state if omitted)'),
    },
  },
  async ({ noun, id, status }) => {
    if (GRAPHDL_MODE === 'local') {
      const transitions = await systemCall(`transitions:${noun}`, status || '')
      const raw = await systemCall(`get:${noun}`, id)
      try {
        return textResult({
          entity: id,
          noun,
          transitions: transitions,
          // Navigation links are part of the entity representation
          entity_data: JSON.parse(raw),
        })
      } catch {
        return textResult({ entity: id, transitions, raw })
      }
    }
    const data = await httpRequest(`/arest/default/${encodeURIComponent(noun)}/${encodeURIComponent(id)}/actions`)
    return textResult(data)
  },
)

// ── 5. explain: derivation trace for a fact or entity ────────────────

server.registerTool(
  'explain',
  {
    description: 'Explain how a fact was derived or why an entity is in its current state. Returns the derivation chain: which rules fired, in what order, producing which facts. Also shows the audit trail for the entity.',
    inputSchema: {
      id: z.string().describe('Entity ID'),
      noun: z.string().optional().describe('Entity noun type'),
      fact: z.string().optional().describe('Specific fact to explain (e.g. "status", "Hypothesis_explains_Observation")'),
    },
  },
  async ({ id, noun, fact }) => {
    if (GRAPHDL_MODE === 'local') {
      // Audit trail for this entity
      const auditRaw = await systemCall('audit_log', '0')
      let audit: any[] = []
      try { audit = JSON.parse(auditRaw) } catch {}

      // If a specific fact type is requested, query it
      let factData = null
      if (fact) {
        const raw = await systemCall(`query:${fact}`, JSON.stringify(noun ? { [noun]: id } : {}))
        try { factData = JSON.parse(raw) } catch { factData = raw }
      }

      return textResult({
        entity: id,
        fact_query: factData,
        audit_trail: Array.isArray(audit) ? audit.filter((a: any) => a?.entity === id || a?.resource === id) : audit,
      })
    }
    const data = await httpRequest(`/arest/default/explain/${encodeURIComponent(id)}`)
    return textResult(data)
  },
)

// ── 6. compile: ingest FORML2 readings (self-modification) ───────────

server.registerTool(
  'compile',
  {
    description: 'Compile FORML2 readings into the engine (self-modification, Corollary 5). The engine extends its own program. New nouns, fact types, constraints, derivation rules, and state machines become active immediately. Alethic violations reject.',
    inputSchema: {
      readings: z.string().describe('FORML2 readings as markdown text'),
    },
  },
  async ({ readings }) => {
    if (GRAPHDL_MODE === 'local') {
      const raw = await systemCall('compile', readings)
      return textResult({ ok: !raw.startsWith('⊥'), result: raw })
    }
    const data = await httpRequest('/parse', {
      method: 'POST',
      body: JSON.stringify({ text: readings }),
    })
    return textResult(data)
  },
)

// ── Utility: schema ──────────────────────────────────────────────────

server.registerTool(
  'schema',
  {
    description: 'Get the full schema: nouns, fact types, constraints, state machines, derivation rules.',
  },
  async () => {
    if (GRAPHDL_MODE === 'local') {
      const data = await dispatchRead('/schema')
      return textResult(data)
    }
    const data = await httpRequest('/arest/default/schema')
    return textResult(data)
  },
)

// ── Utility: verify signature ────────────────────────────────────────

server.registerTool(
  'verify',
  {
    description: 'Verify an HMAC-SHA256 signature over sender + payload.',
    inputSchema: {
      sender: z.string().describe('Claimed sender identity'),
      payload: z.string().describe('Signed payload'),
      signature: z.string().describe('Signature to verify'),
    },
  },
  async ({ sender, payload, signature }) => {
    if (GRAPHDL_MODE === 'local') {
      const encoded = `<${sender},${payload},${signature}>`
      const raw = await systemCall('verify_signature', encoded)
      return textResult({ valid: raw === 'true' })
    }
    const data = await httpRequest('/crypto/verify', {
      method: 'POST',
      body: JSON.stringify({ sender, payload, signature }),
    })
    return textResult(data)
  },
)

// =====================================================================
// LLM BRIDGE — natural-language ↔ formal facts via client sampling
// =====================================================================
//
// These tools use MCP sampling (server.server.createMessage) to request
// LLM completions from the CLIENT'S LLM session. The server composes
// prompts using the schema as context, then runs an engine operation
// with the LLM's response. This inverts the usual agent/tool pattern:
// the engine orchestrates LLM reasoning, not the other way around.

/** Helper to extract text from an LLM sampling response. */
function samplingText(response: any): string {
  const content = response.content
  if (Array.isArray(content)) {
    for (const block of content) {
      if (block.type === 'text') return block.text
    }
    return ''
  }
  return content?.type === 'text' ? content.text : ''
}

/** Strip markdown code fences and parse JSON. */
function parseJsonFromLlm(text: string): any {
  const clean = text.replace(/^```(?:json)?\s*/m, '').replace(/\s*```\s*$/m, '').trim()
  return JSON.parse(clean)
}

// ── ask: natural-language query → project → results ──────────────────

server.registerTool(
  'ask',
  {
    description: 'Ask a natural-language question. The engine uses the client LLM to translate the question into a θ₁ projection, executes it against the population, and returns results. Single-call query experience.',
    inputSchema: {
      question: z.string().describe('Natural language question, e.g. "How many orders did acme place this month?"'),
      noun: z.string().optional().describe('Optional scope hint: fact type or entity noun name'),
    },
  },
  async ({ question, noun }) => {
    if (GRAPHDL_MODE !== 'local') {
      return textResult({ error: 'ask requires local mode' })
    }
    // Gather schema context for the LLM
    const schemaRaw = noun
      ? await systemCall(`schema:${noun}`, '')
      : await systemCall('list:Noun', '')

    const prompt = `You are translating a natural-language question into a projection query.

Schema:
${schemaRaw}

Question: ${question}

Respond with JSON ONLY in this format:
{"fact_type": "Fact_Type_Name", "filter": {"role1": "value1"}}

Use the exact fact_type names from the schema. Leave filter empty {} if no specific constraint. Do not include explanations.`

    let llmResponse
    try {
      llmResponse = await (server as any).server.createMessage({
        messages: [{ role: 'user', content: { type: 'text', text: prompt } }],
        maxTokens: 500,
      })
    } catch (e: any) {
      return textResult({
        error: 'LLM sampling unavailable. Client may not support sampling.',
        details: String(e?.message || e),
      })
    }

    const specText = samplingText(llmResponse)
    let spec
    try {
      spec = parseJsonFromLlm(specText)
    } catch {
      return textResult({
        error: 'LLM did not return valid JSON projection spec',
        llm_response: specText,
      })
    }

    // Execute the projection
    const filterStr = Object.entries(spec.filter || {})
      .map(([k, v]) => `<${k},${v}>`).join('')
    const raw = await systemCall(`query:${spec.fact_type}`, filterStr)
    let results: any
    try { results = JSON.parse(raw) } catch { results = { raw } }

    return textResult({ question, query: spec, results })
  },
)

// ── synthesize: fact bag → derive + verbalize → prose ────────────────

server.registerTool(
  'synthesize',
  {
    description: 'Turn facts about an entity into natural-language prose. Runs the full pipeline (resolve + derive to LFP + validate) to include implicit/derived facts, then verbalizes via the client LLM. The engine guarantees content correctness; the LLM shapes the prose.',
    inputSchema: {
      noun: z.string().describe('Entity noun, e.g. "Order"'),
      id: z.string().optional().describe('Specific entity ID, or synthesize all entities of the noun if omitted'),
    },
  },
  async ({ noun, id }) => {
    if (GRAPHDL_MODE !== 'local') {
      return textResult({ error: 'synthesize requires local mode' })
    }
    // Fetch entity data (includes derived facts — get uses the full pipeline)
    const raw = id
      ? await systemCall(`get:${noun}`, id)
      : await systemCall(`list:${noun}`, '')
    let data: any
    try { data = JSON.parse(raw) } catch { data = { raw } }

    const prompt = `Write a clear, natural-language summary of this information. Use only the facts given. Do not invent details. Prefer direct, declarative prose. Keep it concise.

Entity: ${noun}${id ? ` "${id}"` : ' (all instances)'}

Facts:
${JSON.stringify(data, null, 2)}`

    let llmResponse
    try {
      llmResponse = await (server as any).server.createMessage({
        messages: [{ role: 'user', content: { type: 'text', text: prompt } }],
        maxTokens: 1000,
      })
    } catch (e: any) {
      return textResult({
        error: 'LLM sampling unavailable. Client may not support sampling.',
        details: String(e?.message || e),
        facts: data,
      })
    }

    const prose = samplingText(llmResponse)
    return textResult({ noun, id, facts: data, prose })
  },
)

// ── validate: raw text → extract facts → constraint check ────────────

server.registerTool(
  'validate',
  {
    description: 'Check whether raw text violates a deontic OWA constraint. The client LLM extracts fact instances from the text matching the constraint\'s fact types, then the engine verifies those facts against the constraint. Useful for document review and content moderation.',
    inputSchema: {
      text: z.string().describe('Raw text to check'),
      constraint: z.string().describe('Constraint ID (from compiled defs) or the constraint reading text'),
    },
  },
  async ({ text, constraint }) => {
    if (GRAPHDL_MODE !== 'local') {
      return textResult({ error: 'validate requires local mode' })
    }
    // Get constraint context (fact types it spans, reading text)
    const constraintRaw = await systemCall(`constraint:${constraint}`, '').catch(() => '')

    const prompt = `Extract fact instances from the text that are relevant to the given constraint.

Constraint: ${constraintRaw || constraint}

Text to check:
${text}

Respond with JSON ONLY as an array of facts:
[{"fact_type": "Fact_Type_Name", "bindings": {"role1": "value1"}}, ...]

Only include facts clearly stated or strongly implied by the text. Do not invent. Return [] if no relevant facts are present.`

    let llmResponse
    try {
      llmResponse = await (server as any).server.createMessage({
        messages: [{ role: 'user', content: { type: 'text', text: prompt } }],
        maxTokens: 1500,
      })
    } catch (e: any) {
      return textResult({
        error: 'LLM sampling unavailable. Client may not support sampling.',
        details: String(e?.message || e),
      })
    }

    const extractedText = samplingText(llmResponse)
    let facts: any
    try {
      facts = parseJsonFromLlm(extractedText)
    } catch {
      return textResult({
        error: 'LLM did not return valid JSON facts array',
        llm_response: extractedText,
      })
    }

    // Run verify (dry-run) against each extracted fact.
    // The engine returns violations without mutating state.
    const violations: any[] = []
    for (const fact of Array.isArray(facts) ? facts : []) {
      const bindings = fact.bindings || {}
      const factStr = Object.entries(bindings)
        .map(([k, v]) => `<${k},${v}>`).join('')
      try {
        const vraw = await systemCall(`verify:${fact.fact_type}`, factStr)
        const result = (() => { try { return JSON.parse(vraw) } catch { return { raw: vraw } } })()
        if (result.violations && result.violations.length > 0) {
          violations.push({ fact, violations: result.violations })
        }
      } catch (e: any) {
        violations.push({ fact, error: String(e?.message || e) })
      }
    }

    return textResult({
      text,
      constraint,
      extracted_facts: facts,
      violations,
      satisfied: violations.length === 0,
    })
  },
)

// ── Debug (gated) ────────────────────────────────────────────────────

if (GRAPHDL_DEBUG) {
  server.registerTool(
    'debug',
    { description: 'Dump full compiled state. Development only — GRAPHDL_DEBUG=1.' },
    async () => {
      if (GRAPHDL_MODE === 'local') {
        const raw = await systemCall('debug', '')
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
