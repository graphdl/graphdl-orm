/**
 * Remote MCP server for Worker deployment.
 *
 * Exposes the core AREST verbs over the Streamable HTTP transport so
 * ChatGPT (and any MCP client that supports remote servers) can connect
 * to a public endpoint with a bearer token. The Cloudflare Worker's
 * single WASM isolate holds one engine handle; the metamodel is loaded
 * at first use and cached. New readings arrive via the `compile` tool.
 *
 * Tools intentionally mirror src/mcp/server.ts (the stdio server)
 * minus the fs-dependent `tutor` tool and with the same prompt-only
 * fallback shape for LLM-bridge verbs (ask/synthesize/validate) — a
 * remote MCP server can't initiate client sampling back through an
 * HTTP transport without coordination, so those verbs always return
 * the prompt for the caller to run.
 *
 * v1 scope: single shared handle per isolate. Every connected client
 * sees the same D. Multi-tenant isolation (session → handle) is a
 * follow-up.
 */

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { WebStandardStreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js'
import { z } from 'zod'

import { getEngine, getHandle, systemCall, safeJson } from './engine-bridge.js'

function textResult(data: any) {
  return { content: [{ type: 'text' as const, text: JSON.stringify(data, null, 2) }] }
}

/** Build a fresh McpServer bound to the shared Worker engine handle. */
export function createRemoteServer(): McpServer {
  const server = new McpServer({ name: 'arest-remote', version: '0.2.0' })

  server.registerTool(
    'schema',
    {
      description: 'Get the full schema: nouns, fact types, constraints, state machines, derivation rules.',
      inputSchema: {},
    },
    async () => {
      const raw = await systemCall('debug', '')
      return textResult(safeJson(raw, { raw }))
    },
  )

  // ── ChatGPT compatibility (search + fetch) ────────────────────────────
  // OpenAI's deep research, company knowledge, and ChatGPT-as-app modes
  // require these two specific tools. The contract is documented at
  // https://developers.openai.com/apps-sdk/build/mcp-server. The
  // implementations adapt AREST's entity model: ids round-trip as
  // "Noun:entityId" so fetch can route by noun without state.

  function chatgptResult(payload: any) {
    return { content: [{ type: 'text' as const, text: JSON.stringify(payload) }] }
  }

  function knownNouns(): string[] {
    const dump = safeJson(systemCallSync('debug', ''), {})
    return Object.keys((dump as any).nouns ?? {})
  }

  // systemCall is async, but for search we want to fan out lookups in
  // parallel without churning through `await` in a loop. Engine calls are
  // in-process (WASM in the same isolate), so a sync wrapper is fine.
  function systemCallSync(key: string, input: string): string {
    if (!_engine) throw new Error('engine not initialized')
    if (_handle === null) throw new Error('handle not allocated')
    return _engine.system(_handle, key, input)
  }

  server.registerTool(
    'search',
    {
      description: 'Search the population for entities whose fields match a query string. Returns ChatGPT-compatible {results: [{id, title, url}]}. The id is "Noun:entityId" and round-trips into `fetch`.',
      inputSchema: { query: z.string() },
    },
    async ({ query }) => {
      await getHandle() // ensure engine warm
      const needle = (query ?? '').trim().toLowerCase()
      if (!needle) return chatgptResult({ results: [] })

      const nouns = knownNouns()
      const results: Array<{ id: string; title: string; url: string }> = []
      const cap = 50

      for (const noun of nouns) {
        if (results.length >= cap) break
        const list = safeJson(systemCallSync(`list:${noun}`, ''), [])
        if (!Array.isArray(list)) continue
        for (const entity of list) {
          if (results.length >= cap) break
          if (!entity || typeof entity !== 'object') continue
          const flat = JSON.stringify(entity).toLowerCase()
          if (!flat.includes(needle) && !noun.toLowerCase().includes(needle)) continue
          const entityId = (entity as any).id ?? ''
          if (!entityId) continue
          results.push({
            id: `${noun}:${entityId}`,
            title: `${noun} ${entityId}`,
            url: `/api/entities/${encodeURIComponent(noun)}/${encodeURIComponent(entityId)}`,
          })
        }
      }
      return chatgptResult({ results })
    },
  )

  server.registerTool(
    'fetch',
    {
      description: 'Fetch one entity by id (the "Noun:entityId" form returned by `search`). Returns ChatGPT-compatible {id, title, text, url, metadata}.',
      inputSchema: { id: z.string() },
    },
    async ({ id }) => {
      await getHandle()
      const sep = id.indexOf(':')
      if (sep < 0) {
        return chatgptResult({ id, title: id, text: '', url: '', metadata: { error: 'id must be of the form "Noun:entityId"' } })
      }
      const noun = id.slice(0, sep)
      const entityId = id.slice(sep + 1)
      const entity = safeJson(systemCallSync(`get:${noun}`, entityId), null)
      const sm = safeJson(systemCallSync('get:State Machine', entityId), null) as any
      const status = sm && typeof sm.currentlyInStatus === 'string' ? sm.currentlyInStatus : null
      return chatgptResult({
        id,
        title: `${noun} ${entityId}`,
        text: entity ? JSON.stringify(entity, null, 2) : 'No entity found.',
        url: `/api/entities/${encodeURIComponent(noun)}/${encodeURIComponent(entityId)}`,
        metadata: { noun, entityId, status },
      })
    },
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
      if (!noun) return textResult({ error: 'Provide noun to get or list.' })
      const raw = id
        ? await systemCall(`get:${noun}`, id)
        : await systemCall(`list:${noun}`, '')
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
      const raw = await systemCall(`query:${fact_type}`, filterStr)
      return textResult(safeJson(raw, []) ?? [])
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
        const raw = await systemCall(`transition:${noun}`, `<${id}, ${event}>`)
        return textResult(safeJson(raw, { raw }))
      }
      const pairs = Object.entries(fields || {}).map(([k, v]) => `<${k}, ${v}>`).join(', ')
      const idPair = id ? `<id, ${id}>, ` : ''
      const key = operation === 'create' ? `create:${noun}` : `update:${noun}`
      const raw = await systemCall(key, `<${idPair}${pairs}>`)
      return textResult(safeJson(raw, { raw }))
    },
  )

  server.registerTool(
    'actions',
    {
      description: 'Get valid SM transitions and navigation for an entity. Pure HATEOAS.',
      inputSchema: { noun: z.string(), id: z.string(), status: z.string().optional() },
    },
    async ({ noun, id, status }) => {
      let resolvedStatus = status || ''
      if (!resolvedStatus) {
        const sm = safeJson(await systemCall('get:State Machine', id), null)
        if (sm && typeof sm.currentlyInStatus === 'string') resolvedStatus = sm.currentlyInStatus
      }
      const transitions = safeJson(await systemCall(`transitions:${noun}`, resolvedStatus), [])
      const entity = safeJson(await systemCall(`get:${noun}`, id), null)
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
      const raw = await systemCall('compile', readings)
      const ok = !raw.startsWith('⊥')
      return textResult({ ok, result: safeJson(raw, raw) })
    },
  )

  server.registerTool(
    'explain',
    {
      description: 'Explain how an entity reached its current state: derivation chain + audit trail.',
      inputSchema: { id: z.string(), noun: z.string().optional(), fact: z.string().optional() },
    },
    async ({ id, noun, fact }) => {
      let audit: any[] = []
      const parsed = safeJson(await systemCall('audit', '0'), [])
      if (Array.isArray(parsed)) audit = parsed
      let factData: any = []
      if (fact) {
        const raw = await systemCall(`query:${fact}`, JSON.stringify(noun ? { [noun]: id } : {}))
        factData = safeJson(raw, []) ?? []
      }
      return textResult({
        entity: id,
        fact_query: factData,
        audit_trail: audit.filter((a: any) => a?.entity === id || a?.resource === id),
      })
    },
  )

  // LLM-bridge verbs. Remote transports can't round-trip client sampling,
  // so these always return a prompt-only payload the caller runs itself
  // and re-invokes with `llm_response`.
  server.registerTool(
    'ask',
    {
      description: 'Translate a natural-language question into a projection, execute it, return matching facts. On a remote server the prompt is returned for manual execution unless `llm_response` is supplied.',
      inputSchema: {
        question: z.string(),
        noun: z.string().optional(),
        llm_response: z.string().optional(),
      },
    },
    async ({ question, noun, llm_response }) => {
      const schemaRaw = noun
        ? await systemCall(`schema:${noun}`, '')
        : await systemCall('list:Noun', '')
      const prompt = `Translate the question into a projection query.\n\nSchema:\n${schemaRaw}\n\nQuestion: ${question}\n\nRespond with JSON ONLY: {"fact_type": "...", "filter": {...}}`
      if (!llm_response) {
        return textResult({
          mode: 'prompt-only',
          reason: 'remote MCP does not round-trip client sampling',
          prompt,
          next_step: 'Run this prompt against any LLM, then re-invoke `ask` with the result as `llm_response`.',
          question,
        })
      }
      let spec
      try { spec = JSON.parse(llm_response.replace(/^```(?:json)?\s*/m, '').replace(/\s*```\s*$/m, '')) }
      catch { return textResult({ error: 'llm_response is not valid JSON', llm_response }) }
      const filterStr = Object.entries(spec.filter || {}).map(([k, v]) => `<${k},${v}>`).join('')
      const raw = await systemCall(`query:${spec.fact_type}`, filterStr)
      return textResult({ question, query: spec, results: safeJson(raw, []) ?? [] })
    },
  )

  return server
}

/** Handle an MCP request on the Worker /mcp route. */
export async function handleMcpRequest(request: Request): Promise<Response> {
  const server = createRemoteServer()
  const transport = new WebStandardStreamableHTTPServerTransport({
    sessionIdGenerator: undefined, // stateless for v1
  })
  await server.connect(transport)
  return transport.handleRequest(request)
}
