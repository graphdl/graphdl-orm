/**
 * `/arest/chat` — Worker-side handler for the `chat` agent verb
 * (#640 / Worker-AI-3).
 *
 * Direct mirror of `./extract.ts` (#639 / Worker-AI-2) — same
 * dispatch-through-engine `Func::Def` chain, same envelope contract,
 * same fail-closed semantics. The only behavioural divergence between
 * the two handlers is at step 5 of the dispatch path:
 *
 *   * `extract` expects strict-JSON LLM output and falls back to a
 *     `_raw` field on parse failure.
 *   * `chat` expects natural-prose output and surfaces the LLM text
 *     verbatim in the envelope's `text` field. No JSON parsing is
 *     attempted — chat is conversational, not structured.
 *
 * Dispatch path
 * -------------
 *
 *   1. Parse JSON body.
 *   2. Resolve the Agent Definition for verb "chat" via the cell
 *      walker `resolveAgentVerb` (re-exported from `./extract.ts`).
 *      Returns `{ modelCode, prompt, agentDefinitionId }` or `null`.
 *   3. On `null` → 503 with the `chat.no_body` envelope shape the
 *      kernel emits for the same failure mode (#620 / HATEOAS-6b).
 *      Mirror of `extract.no_body` so HATEOAS-aware clients can
 *      branch on a single envelope schema across both verbs.
 *   4. Otherwise render the prompt template with the request body as
 *      input and call `aiComplete(prompt, { env, model })`.
 *   5. On `aiComplete` returning `{ error }` → 503 with the underlying
 *      error code bubbled up as `chat.ai_complete.<code>`.
 *   6. On `aiComplete` returning `{ text }` → 200 with `{ text,
 *      _meta?, citations? }` — chat output is always natural prose,
 *      so no JSON parse step (unlike `extract`).
 *
 * The handler NEVER throws — every failure is a structured envelope
 * the engine can map to `Object::Bottom`.
 *
 * Agent Definition seeding
 * ------------------------
 *
 * The `chat` Agent Definition (id `agent-chatter`, name "Chatter") is
 * seeded by #641 / Worker-AI-4 in `./agent-seed.ts` alongside
 * `Extractor`. Until #641 lands the seed, every real request hits the
 * 503 path — same correct-but-unconfigured contract as the pre-#641
 * `/arest/extract`. Tests inject a fully populated state via
 * `opts.state` to verify both branches.
 */

import type { AiCompleteEnv, AiCompleteResult } from './complete'
import { aiComplete as aiCompleteImpl } from './complete'
import {
  resolveAgentVerb,
  type AgentVerbState,
} from './extract'

/**
 * Worker URL the `Retry-After` header points the caller at when
 * `POST /arest/chat` falls into a failure path. Mirror of
 * `EXTRACT_WORKER_URL` for the chat verb. Single source of truth so
 * the header value, the envelope's `_links.worker.href`, and the
 * envelope's top-level `retryAfter` can't drift.
 */
export const CHAT_WORKER_URL = 'https://arest.do/arest/chat'

// ── Public types ────────────────────────────────────────────────────

export interface ChatHandlerOptions {
  /**
   * Boot state with the agent metamodel cells. In production this is
   * populated by #641's seeding path (`AGENT_DEFINITIONS_STATE`); in
   * tests it's injected directly. Defaults to `{}` (the pre-#641 boot
   * state — every request 503s).
   */
  readonly state?: AgentVerbState
  /**
   * Override for the LLM call. Defaults to the production `aiComplete`
   * import; tests inject a mock so they never hit the network.
   */
  readonly aiComplete?: (
    prompt: string,
    opts: { env: AiCompleteEnv; model?: string },
  ) => Promise<AiCompleteResult>
}

// ── Helpers ─────────────────────────────────────────────────────────

/**
 * Best-effort Model.code lookup for envelope introspection. Mirror of
 * `extract.ts`'s `introspectAgentDefinition` for the chat verb.
 *
 * Used on the 503 path so the envelope's `agentDefinition` field can
 * surface a partially configured Agent Definition (e.g. Model present
 * but Prompt missing) — same introspection leak the kernel #620
 * envelope already permits.
 *
 * Returns `null` when the verb has no AgentDef binding at all.
 */
function introspectAgentDefinition(
  state: AgentVerbState,
  verbName: string,
): { model?: string; prompt?: string } | null {
  const verbs = state.Verb ?? []
  const verbId = verbs.find((v) => v.name === verbName)?.id
  if (!verbId) return null
  const invokes = state.Verb_invokes_Agent_Definition ?? []
  const agentId = invokes.find((f) => f.Verb === verbId)?.['Agent Definition']
  if (!agentId) return null
  const model = (state.Agent_Definition_uses_Model ?? [])
    .find((f) => f['Agent Definition'] === agentId)?.Model
  const prompt = (state.Agent_Definition_has_Prompt ?? [])
    .find((f) => f['Agent Definition'] === agentId)?.Prompt
  return { model, prompt }
}

/**
 * Render the agent's prompt template with the request body as input.
 * Same minimal templating as `extract.ts::renderPrompt` — once a real
 * templating spec lands (placeholder substitution, role-aware
 * messages), both handlers move to a shared module.
 */
function renderPrompt(template: string, body: unknown): string {
  return `${template}\n\nInput:\n${JSON.stringify(body, null, 2)}`
}

/**
 * Build the 503 envelope per #620's spec. Mirrors `extract.ts`'s
 * `buildEnvelope` exactly — same field shape so HATEOAS-aware clients
 * can branch on a single envelope schema across both verbs and across
 * the kernel/worker targets.
 */
function buildEnvelope(
  code: string,
  message: string,
  introspection: { model?: string; prompt?: string } | null,
): Record<string, unknown> {
  const errorEntry: Record<string, unknown> = {
    code,
    message,
    verb: 'chat',
    _links: { worker: { href: CHAT_WORKER_URL } },
  }
  if (introspection && (introspection.model || introspection.prompt)) {
    errorEntry.agentDefinition = {
      ...(introspection.model !== undefined && { model: introspection.model }),
      ...(introspection.prompt !== undefined && { prompt: introspection.prompt }),
    }
  }
  return {
    errors: [errorEntry],
    status: 503,
    retryAfter: CHAT_WORKER_URL,
  }
}

function envelope503(body: Record<string, unknown>): Response {
  return new Response(JSON.stringify(body), {
    status: 503,
    headers: {
      'content-type': 'application/json',
      'retry-after': CHAT_WORKER_URL,
    },
  })
}

// ── Handler ─────────────────────────────────────────────────────────

/**
 * Main handler. The router calls this from the `/arest/chat` route;
 * the function is also re-exportable for the engine-side platform-fn
 * install path once that wire lands.
 */
export async function handleChat(
  request: Request,
  env: AiCompleteEnv,
  opts: ChatHandlerOptions = {},
): Promise<Response> {
  const state = opts.state ?? {}
  const aiComplete = opts.aiComplete ?? aiCompleteImpl

  // 1. Parse the request body. Malformed JSON falls into `chat.parse`
  //    so a bad POST never panics — mirror of the kernel branch.
  let body: unknown
  try {
    body = await request.json()
  } catch {
    return envelope503(
      buildEnvelope(
        'chat.parse',
        "Request body did not parse as JSON; nothing to dispatch to the 'chat' verb.",
        null,
      ),
    )
  }

  // 2. Resolve the Agent Definition. `null` → 503 with introspectable
  //    metadata (when partial config is reachable via the four-cell
  //    walk).
  const binding = resolveAgentVerb(state, 'chat')
  if (!binding) {
    return envelope503(
      buildEnvelope(
        'chat.no_body',
        "The 'chat' verb is registered on this worker but no Agent Definition is " +
          'installed for it. Seed the Agent Definition cells (Verb, ' +
          'Verb_invokes_Agent_Definition, Agent_Definition_uses_Model, ' +
          'Agent_Definition_has_Prompt) per readings/templates/agents.md, then retry.',
        introspectAgentDefinition(state, 'chat'),
      ),
    )
  }

  // 3. Render the prompt template + dispatch through aiComplete.
  const rendered = renderPrompt(binding.prompt, body)
  const result = await aiComplete(rendered, { env, model: binding.modelCode })

  // 4. aiComplete failure → 503 with the underlying code bubbled up.
  if ('error' in result) {
    return envelope503({
      errors: [{
        code: `chat.ai_complete.${result.error.code}`,
        message: result.error.message,
        verb: 'chat',
        agentDefinition: {
          model: binding.modelCode,
          // Don't echo the prompt body in the error envelope — it can
          // be huge and contain user data. The `agentDefinitionId`
          // gives the dashboard enough to fetch the full prompt via
          // `/explain` if needed.
          agentDefinitionId: binding.agentDefinitionId,
        },
        _links: { worker: { href: CHAT_WORKER_URL } },
      }],
      status: 503,
      retryAfter: CHAT_WORKER_URL,
    })
  }

  // 5. Success — surface the LLM text verbatim. Unlike `extract`, the
  //    chat verb's contract is "produce natural prose", so no JSON
  //    parse step. `_meta` carries provenance (model + gateway +
  //    finishReason) for engine-side Citation emission; `citations`
  //    is reserved for tool-output / RAG citations once that wire
  //    lands.
  return new Response(
    JSON.stringify({
      text: result.text,
      _meta: result._meta,
      ...(result.citations !== undefined && { citations: result.citations }),
      _links: { self: { href: CHAT_WORKER_URL } },
    }),
    {
      status: 200,
      headers: { 'content-type': 'application/json' },
    },
  )
}
