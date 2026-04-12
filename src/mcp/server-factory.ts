/**
 * Build an McpServer with the core AREST verbs wired up. Transport-agnostic:
 * call createArestServer(), then connect whatever transport fits (stdio for
 * Claude Desktop / Claude Code, Streamable HTTP for remote, or something
 * you write yourself).
 *
 * Consumers on npm:
 *
 *   import { createArestServer } from 'arest'
 *   import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
 *
 *   const server = createArestServer({ readings: [myDomain] })
 *   await server.connect(new StdioServerTransport())
 */

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { z } from 'zod'
import * as engine from '../api/engine.js'

export interface CreateArestServerOptions {
  /** Existing engine handle to use. Omit to allocate a fresh one with the metamodel loaded. */
  handle?: number
  /** FORML2 readings to compile into the engine on startup. Appended to the metamodel. */
  readings?: string[]
  /** MCP server name (shown to clients). Default: "arest". */
  name?: string
  /** MCP server version (shown to clients). Default: "0.7.0". */
  version?: string
}

function textResult(data: any) {
  return { content: [{ type: 'text' as const, text: JSON.stringify(data, null, 2) }] }
}

function safeJson<T>(raw: string, fallback: T): T | any {
  try { const v = JSON.parse(raw); return v ?? fallback } catch { return fallback }
}

/**
 * Create an MCP server with the core AREST verbs registered. Returns the
 * server unconnected; the caller attaches a transport.
 *
 * Verbs registered: schema, get, query, apply, actions, compile, explain,
 * ask. The LLM-bridge verb (ask) attempts MCP client sampling and falls
 * back to returning the prompt for manual execution if sampling isn't
 * available on the connected transport.
 */
export function createArestServer(options: CreateArestServerOptions = {}): McpServer {
  const handle = options.handle ?? engine.compileDomainReadings(...(options.readings ?? []))
  const sys = (key: string, input: string) => engine.system(handle, key, input)

  const server = new McpServer({
    name: options.name ?? 'arest',
    version: options.version ?? '0.7.0',
  })

  server.registerTool(
    'schema',
    {
      description: 'Get the full schema: nouns, fact types, constraints, state machines, derivation rules.',
      inputSchema: {},
    },
    async () => textResult(safeJson(sys('debug', ''), { raw: sys('debug', '') })),
  )

  server.registerTool(
    'get',
    {
      description: 'Get an entity by ID, or list all entities of a noun type.',
      inputSchema: {
        noun: z.string().optional(),
        id: z.string().optional(),
      },
    },
    async ({ noun, id }) => {
      if (!noun) return textResult({ error: 'Provide a noun to get or list.' })
      const raw = id ? sys(`get:${noun}`, id) : sys(`list:${noun}`, '')
      return textResult(safeJson(raw, { raw }))
    },
  )

  server.registerTool(
    'query',
    {
      description: 'Query facts by fact type with optional role-binding filter.',
      inputSchema: {
        fact_type: z.string(),
        filter: z.record(z.string(), z.string()).optional(),
      },
    },
    async ({ fact_type, filter }) => {
      const filterStr = filter ? JSON.stringify(filter) : ''
      return textResult(safeJson(sys(`query:${fact_type}`, filterStr), []) ?? [])
    },
  )

  server.registerTool(
    'apply',
    {
      description: 'Apply an operation to an entity (create / update / transition). Runs the full pipeline: resolve -> derive -> validate -> emit.',
      inputSchema: {
        operation: z.enum(['create', 'update', 'transition']),
        noun: z.string(),
        id: z.string().optional(),
        event: z.string().optional(),
        fields: z.record(z.string(), z.string()).optional(),
      },
    },
    async ({ operation, noun, id, event, fields }) => {
      if (operation === 'transition') {
        if (!id || !event) return textResult({ error: 'transition requires id and event' })
        return textResult(safeJson(sys(`transition:${noun}`, `<${id}, ${event}>`), { raw: '' }))
      }
      const pairs = Object.entries(fields || {}).map(([k, v]) => `<${k}, ${v}>`).join(', ')
      const idPair = id ? `<id, ${id}>, ` : ''
      const key = operation === 'create' ? `create:${noun}` : `update:${noun}`
      return textResult(safeJson(sys(key, `<${idPair}${pairs}>`), { raw: '' }))
    },
  )

  server.registerTool(
    'actions',
    {
      description: 'Valid SM transitions and navigation links for an entity. Pure HATEOAS.',
      inputSchema: { noun: z.string(), id: z.string(), status: z.string().optional() },
    },
    async ({ noun, id, status }) => {
      let resolvedStatus = status ?? ''
      if (!resolvedStatus) {
        const sm: any = safeJson(sys('get:State Machine', id), null)
        if (sm && typeof sm.currentlyInStatus === 'string') resolvedStatus = sm.currentlyInStatus
      }
      const transitions = safeJson(sys(`transitions:${noun}`, resolvedStatus), []) ?? []
      const entity = safeJson(sys(`get:${noun}`, id), null)
      return textResult({ entity: id, noun, status: resolvedStatus || null, transitions, entity_data: entity })
    },
  )

  server.registerTool(
    'compile',
    {
      description: 'Compile FORML2 readings into the running engine (self-modification).',
      inputSchema: { readings: z.string() },
    },
    async ({ readings }) => {
      const raw = sys('compile', readings)
      return textResult({ ok: !raw.startsWith('⊥'), result: safeJson(raw, raw) })
    },
  )

  server.registerTool(
    'explain',
    {
      description: 'Derivation chain and audit trail for an entity.',
      inputSchema: { id: z.string(), noun: z.string().optional(), fact: z.string().optional() },
    },
    async ({ id, noun, fact }) => {
      const audit = safeJson(sys('audit', '0'), [])
      const factData = fact
        ? (safeJson(sys(`query:${fact}`, JSON.stringify(noun ? { [noun]: id } : {})), []) ?? [])
        : []
      return textResult({
        entity: id,
        fact_query: factData,
        audit_trail: Array.isArray(audit)
          ? audit.filter((a: any) => a?.entity === id || a?.resource === id)
          : [],
      })
    },
  )

  server.registerTool(
    'ask',
    {
      description: 'Translate a natural-language question into a projection, execute it, return matching facts. Tries MCP client sampling; falls back to returning the prompt for manual execution if the client doesn\'t support sampling. Pass llm_response to supply a pre-sampled answer.',
      inputSchema: {
        question: z.string(),
        noun: z.string().optional(),
        llm_response: z.string().optional(),
      },
    },
    async ({ question, noun, llm_response }) => {
      const schemaRaw = noun ? sys(`schema:${noun}`, '') : sys('list:Noun', '')
      const prompt = `Translate the question into a projection query.\n\nSchema:\n${schemaRaw}\n\nQuestion: ${question}\n\nRespond with JSON ONLY: {"fact_type": "...", "filter": {...}}`

      let answer = llm_response
      if (!answer) {
        try {
          const res: any = await (server as any).server.createMessage({
            messages: [{ role: 'user', content: { type: 'text', text: prompt } }],
            maxTokens: 500,
          })
          const blocks = Array.isArray(res.content) ? res.content : [res.content]
          answer = blocks.find((b: any) => b.type === 'text')?.text ?? ''
        } catch {
          return textResult({
            mode: 'prompt-only',
            reason: 'transport does not support MCP sampling',
            prompt,
            next_step: 'Run the prompt against any LLM, then re-invoke `ask` with the result as `llm_response`.',
            question,
          })
        }
      }

      if (!answer) return textResult({ error: 'No LLM response available' })
      let spec: any
      try { spec = JSON.parse(answer.replace(/^```(?:json)?\s*/m, '').replace(/\s*```\s*$/m, '')) }
      catch { return textResult({ error: 'LLM did not return valid JSON projection spec', llm_response: answer }) }
      const filterStr = Object.entries(spec.filter || {}).map(([k, v]) => `<${k},${v}>`).join('')
      const raw = sys(`query:${spec.fact_type}`, filterStr)
      return textResult({ question, query: spec, results: safeJson(raw, []) ?? [] })
    },
  )

  return server
}
