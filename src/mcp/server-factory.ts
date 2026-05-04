/**
 * Build an McpServer with the core AREST verbs wired up. Transport-agnostic:
 * call createArestServer(), then connect whatever transport fits (stdio for
 * Claude Desktop / Claude Code, Streamable HTTP for remote, or something
 * you write yourself).
 *
 * All tools except `ask` delegate to `dispatchVerb` (#200) so the MCP
 * surface and the HTTP surface share a single implementation. `ask` is
 * the exception because it uses MCP client sampling, which has no HTTP
 * equivalent.
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
import { dispatchVerb } from '../api/verb-dispatcher.js'
import {
  buildMutationContext,
  enforceMutationContext,
  CONTEXT_RECEIPT_FIELD_DESCRIPTION,
  MUTATION_CONTEXT_DESCRIPTION,
  MUTATION_TOOL_DESCRIPTION,
  type MutationContextDetail,
  type MutationContextTool,
} from './mutation-context.js'

export interface CreateArestServerOptions {
  handle?: number
  readings?: string[]
  name?: string
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
 * Every tool (except `ask`) delegates to `dispatchVerb(verb, body, handle)`
 * so behaviour is identical to the HTTP `/api/<verb>` routes (#200).
 */
export function createArestServer(options: CreateArestServerOptions = {}): McpServer {
  const handle = options.handle ?? engine.compileDomainReadings(...(options.readings ?? []))

  const dispatch = async (verb: string, body: Record<string, unknown>) => {
    const envelope = await dispatchVerb(verb, body, handle)
    return textResult(envelope.data)
  }

  const currentMutationContext = (detail: MutationContextDetail = 'summary') => buildMutationContext({ detail })

  const mutationGateResult = (
    tool: MutationContextTool,
    contextReceipt: string | undefined,
    payload: Record<string, unknown>,
  ) => {
    const gate = enforceMutationContext({
      tool,
      receivedReceipt: contextReceipt,
      context: currentMutationContext(),
      payload,
    })
    return gate.ok ? null : textResult(gate.error)
  }

  const server = new McpServer({
    name: options.name ?? 'arest',
    version: options.version ?? '0.7.0',
  })

  // ── Introspection ─────────────────────────────────────────────────

  server.registerTool(
    'context',
    {
      description: MUTATION_CONTEXT_DESCRIPTION,
      inputSchema: {
        detail: z.enum(['summary', 'full']).optional().describe('summary returns rules and prompt digests. full also includes prompt text when available.'),
      },
    },
    async ({ detail }) => textResult(currentMutationContext((detail ?? 'summary') as MutationContextDetail)),
  )

  server.registerTool(
    'schema',
    {
      description: 'Get the full schema: nouns, fact types, constraints, state machines, derivation rules.',
      inputSchema: {},
    },
    async () => dispatch('schema', {}),
  )

  server.registerTool(
    'explain',
    {
      description: 'Derivation chain and audit trail for an entity.',
      inputSchema: { id: z.string(), noun: z.string().optional(), fact: z.string().optional() },
    },
    async (input) => dispatch('explain', input),
  )

  server.registerTool(
    'actions',
    {
      description: 'Valid SM transitions and navigation links for an entity. Pure HATEOAS.',
      inputSchema: { noun: z.string(), id: z.string(), status: z.string().optional() },
    },
    async (input) => dispatch('actions', input),
  )

  // ── Entity CRUD ───────────────────────────────────────────────────

  server.registerTool(
    'get',
    {
      description: 'Get an entity by ID, or list all entities of a noun type.',
      inputSchema: {
        noun: z.string().optional(),
        id: z.string().optional(),
      },
    },
    async (input) => dispatch('get', input),
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
    async (input) => dispatch('query', input),
  )

  server.registerTool(
    'apply',
    {
      description: `Apply an operation to an entity (create / update / transition). Runs the full pipeline: resolve -> derive -> validate -> emit. ${MUTATION_TOOL_DESCRIPTION}`,
      inputSchema: {
        context_receipt: z.string().optional().describe(CONTEXT_RECEIPT_FIELD_DESCRIPTION),
        operation: z.enum(['create', 'update', 'transition']),
        noun: z.string(),
        id: z.string().optional(),
        event: z.string().optional(),
        fields: z.record(z.string(), z.string()).optional(),
      },
    },
    async (input) => {
      const { context_receipt, ...body } = input as Record<string, unknown> & { context_receipt?: string }
      const blocked = mutationGateResult('apply', context_receipt, body)
      if (blocked) return blocked
      return dispatch('apply', body)
    },
  )

  // ── Self-modification ─────────────────────────────────────────────

  server.registerTool(
    'compile',
    {
      description: `Compile FORML2 readings into the running engine (self-modification). ${MUTATION_TOOL_DESCRIPTION}`,
      inputSchema: {
        context_receipt: z.string().optional().describe(CONTEXT_RECEIPT_FIELD_DESCRIPTION),
        readings: z.string(),
      },
    },
    async (input) => {
      const { context_receipt, ...body } = input as Record<string, unknown> & { context_receipt?: string }
      const blocked = mutationGateResult('compile', context_receipt, body)
      if (blocked) return blocked
      return dispatch('compile', body)
    },
  )

  server.registerTool(
    'check',
    {
      description:
        'Diagnose FORML2 readings before compiling. Runs parse, resolve, and deontic layers. Empty diagnostics means the readings are clean. Read-only; no engine state change.',
      inputSchema: { readings: z.string() },
    },
    async (input) => dispatch('check', input),
  )

  server.registerTool(
    'verify',
    {
      description: 'Verify the current domain state for structural soundness.',
      inputSchema: { domain: z.string().optional() },
    },
    async (input) => dispatch('verify', input),
  )

  // ── Persistence ───────────────────────────────────────────────────

  server.registerTool(
    'snapshot',
    {
      description: 'Take a named snapshot of the current engine state.',
      inputSchema: { label: z.string().optional() },
    },
    async (input) => dispatch('snapshot', input),
  )

  server.registerTool(
    'rollback',
    {
      description: 'Rollback the engine to a named snapshot.',
      inputSchema: { label: z.string().optional() },
    },
    async (input) => dispatch('rollback', input),
  )

  server.registerTool(
    'snapshots',
    {
      description: 'List available snapshots.',
      inputSchema: {},
    },
    async () => dispatch('snapshots', {}),
  )

  // ── External System browse (#343) ─────────────────────────────────

  server.registerTool(
    'external_browse',
    {
      description:
        'Browse a type in a mounted External System (e.g. schema.org). Returns {type, supertypes[], subtypes[], properties[{name, range}]}. Inherited properties are included — Person surfaces schema:name via schema:Thing.',
      inputSchema: {
        system: z.string().describe('External System name (e.g. "schema.org")'),
        path: z.array(z.string()).optional()
          .describe('Breadcrumb to the type; last segment picks the type. Empty → system root.'),
      },
    },
    async (input) => dispatch('external_browse', input as Record<string, unknown>),
  )

  // ── LLM bridge (MCP-specific — uses client sampling) ──────────────

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
      const sys = (key: string, input: string) => engine.system(handle, key, input)
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

      const parsed = safeJson(answer!, null)
      if (!parsed || !parsed.fact_type) {
        return textResult({ mode: 'error', reason: 'LLM response did not parse', raw: answer })
      }
      const queryResult = await dispatchVerb('query', {
        fact_type: parsed.fact_type,
        filter: parsed.filter,
      }, handle)
      return textResult(queryResult.data)
    },
  )

  return server
}
