/**
 * GraphDL MCP Server — stdio transport.
 *
 * Standalone process that exposes AREST operations as MCP tools.
 * The API key stays server-side. The AI agent calls tools and gets
 * population data back without seeing credentials.
 *
 * Usage:
 *   GRAPHDL_API_KEY=... GRAPHDL_URL=https://graphdl-orm.dotdo.workers.dev npx tsx src/mcp/server.ts
 *
 * Or via the CLI:
 *   graphdl --mcp
 */

/// <reference types="node" />
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { z } from 'zod'
import { readFileSync } from 'fs'
import { resolve, dirname } from 'path'
import { fileURLToPath } from 'url'

const __dirname = dirname(fileURLToPath(import.meta.url))

const GRAPHDL_URL = process.env.GRAPHDL_URL || 'https://graphdl-orm.dotdo.workers.dev'
const GRAPHDL_API_KEY = process.env.GRAPHDL_API_KEY || ''

async function request(path: string, options?: RequestInit): Promise<any> {
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
    const params = new URLSearchParams()
    if (page) params.set('page', String(page))
    if (limit) params.set('limit', String(limit))
    const qs = params.toString()
    const data = await request(`/arest/${domain}/${encodeURIComponent(noun)}${qs ? '?' + qs : ''}`)
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
    const data = await request(`/arest/${domain}/${encodeURIComponent(noun)}/${encodeURIComponent(id)}`)
    return textResult(data)
  },
)

// ── Create: create a new entity ─────────────────────────────────────

server.tool(
  'graphdl_create',
  'Create a new entity. Returns the created entity with HATEOAS links.',
  {
    noun: z.string().describe('The noun type'),
    domain: z.string().describe('The domain slug'),
    data: z.record(z.string(), z.any()).describe('The entity data'),
  },
  async ({ noun, domain, data: body }) => {
    const result = await request(`/arest/${domain}/${encodeURIComponent(noun)}`, {
      method: 'POST',
      body: JSON.stringify(body),
    })
    return textResult(result)
  },
)

// ── Evaluate: run constraint evaluation ─────────────────────────────

server.tool(
  'graphdl_evaluate',
  'Evaluate constraints against a response. Returns violations.',
  {
    domain: z.string().describe('The domain slug'),
    response: z.record(z.string(), z.any()).describe('The response data to evaluate'),
  },
  async ({ domain, response }) => {
    const data = await request(`/arest/${domain}/evaluate`, {
      method: 'POST',
      body: JSON.stringify({ response }),
    })
    return textResult(data)
  },
)

// ── Schema: get noun schemas for a domain ───────────────────────────

server.tool(
  'graphdl_schema',
  'Get the schema (nouns, fact types, constraints) for a domain',
  {
    domain: z.string().describe('The domain slug'),
  },
  async ({ domain }) => {
    const data = await request(`/arest/${domain}/schema`)
    return textResult(data)
  },
)

// ── Parse: compile ∘ parse readings (self-modification) ─────────────

server.tool(
  'graphdl_parse',
  'Parse FORML2 readings into a domain. compile compose parse. This is self-modification (Corollary 3).',
  {
    domain: z.string().describe('The domain slug'),
    readings: z.string().describe('FORML2 readings as markdown text'),
  },
  async ({ domain, readings }) => {
    const data = await request('/parse', {
      method: 'POST',
      body: JSON.stringify({ domain, text: readings }),
    })
    return textResult(data)
  },
)

// ── Prompts — domain knowledge served on demand ─────────────────────

function loadPrompt(name: string): string {
  return readFileSync(resolve(__dirname, 'prompts', `${name}.md`), 'utf-8')
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
}

main().catch((err) => {
  console.error('GraphDL MCP server failed:', err)
  process.exit(1)
})
