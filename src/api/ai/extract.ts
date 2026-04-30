/**
 * `/arest/extract` — Worker-side handler for the `extract` agent verb
 * (#639 / Worker-AI-2).
 *
 * Dispatch path
 * -------------
 *
 *   1. Parse JSON body.
 *   2. Resolve the Agent Definition for verb "extract" via the cell
 *      walker `resolveAgentVerb` — port of `arest::agent::resolve_agent_verb`
 *      (`crates/arest/src/agent.rs`). Returns `{ modelCode, prompt,
 *      agentDefinitionId }` or `null`.
 *   3. On `null` → 503 with the `extract.no_body` envelope shape the
 *      kernel emits (#620 / HATEOAS-6b). The envelope is introspectable:
 *      the Model.code surfaces in `agentDefinition.model` when
 *      partially configured. Mirrors the kernel exactly so HATEOAS-aware
 *      clients can branch on a single envelope schema across both
 *      targets.
 *   4. Otherwise render the prompt template with the request body as
 *      input and call `aiComplete(prompt, { env, model })`.
 *   5. On `aiComplete` returning `{ error }` → 503 with the underlying
 *      error code bubbled up as `extract.ai_complete.<code>`.
 *   6. On `aiComplete` returning `{ text }` → try `JSON.parse(text)`.
 *      On parse failure surface the raw text in `_raw` so dashboards
 *      can inspect; on success use the parsed object as `data`.
 *
 * The handler NEVER throws — every failure is a structured envelope
 * the engine can map to `Object::Bottom`.
 *
 * Agent Definition seeding (#641 — pending)
 * -----------------------------------------
 *
 * Worker boot state is currently empty for the agent metamodel cells.
 * Until #641 lands and seeds the `extract` Agent Definition, every
 * real request hits the 503 path. The contract is correct — the same
 * handler starts returning 200 once the seed exists. Tests inject a
 * fully populated state via `opts.state` to verify both branches.
 *
 * Why a TS port (vs WASM call)
 * ----------------------------
 *
 * The kernel reaches the resolver via `agent::resolve_agent_verb`
 * over `with_state(...)`. The worker has no equivalent state binding —
 * the engine WASM module exposes `system(h, key, input)` for verbs
 * but no direct cell-walk export. Porting the resolver is ~20 lines
 * (it's a four-cell walker; each cell is a flat list of fact records),
 * which is smaller than wiring a new WASM export. If/when #641 lands
 * with a richer state bridge, the resolver can collapse to a single
 * call into the engine.
 */

import type { AiCompleteEnv, AiCompleteResult } from './complete'
import { aiComplete as aiCompleteImpl } from './complete'

/**
 * Worker URL the `Retry-After` header points the caller at when
 * `POST /arest/extract` falls into the no-body path (#620 / HATEOAS-6b).
 * Single source of truth so the header value, the envelope's
 * `_links.worker.href`, and the envelope's top-level `retryAfter`
 * can't drift. Mirror of `system::EXTRACT_WORKER_URL` in the kernel.
 */
export const EXTRACT_WORKER_URL = 'https://arest.do/arest/extract'

// ── Public types ────────────────────────────────────────────────────

/**
 * State shape the resolver walks. Each field is one cell from
 * `readings/templates/agents.md`, projected to the canonical
 * underscored cell names the parser cascade emits.
 *
 * Each cell is a list of fact records (plain objects). Role bindings
 * are object properties keyed by the role name as it appears in the
 * reading — note `Agent Definition` carries an internal space, so
 * lookups use the literal string. Mirror of the engine's `Object::Seq`
 * fact representation but flattened to TS objects so we can ship
 * them through the worker boot path without a full Object encoder.
 */
export interface AgentVerbState {
  Verb?: ReadonlyArray<Record<string, string>>
  Verb_invokes_Agent_Definition?: ReadonlyArray<Record<string, string>>
  Agent_Definition_uses_Model?: ReadonlyArray<Record<string, string>>
  Agent_Definition_has_Prompt?: ReadonlyArray<Record<string, string>>
  /**
   * Introspection-only cells (#641 / Worker-AI-4). Not consulted by
   * `resolveAgentVerb` — present so dashboards / `/explain` endpoints
   * can surface human-readable Agent Definition metadata, and so the
   * seed shape mirrors the full `readings/templates/agents.md`
   * footprint the kernel populates via `system::init`.
   */
  Agent_Definition_has_Name?: ReadonlyArray<Record<string, string>>
  Agent_Definition_belongs_to_Domain?: ReadonlyArray<Record<string, string>>
}

export interface AgentBinding {
  readonly modelCode: string
  readonly prompt: string
  readonly agentDefinitionId: string
}

export interface ExtractHandlerOptions {
  /**
   * Boot state with the agent metamodel cells. In production this is
   * populated by #641's seeding path; in tests it's injected directly.
   * Defaults to `{}` (the pre-#641 boot state — every request 503s).
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

// ── Resolver ────────────────────────────────────────────────────────

/**
 * Walk `state` to find the Agent Definition that `verbName` invokes,
 * then read its `uses Model` + `has Prompt` facts. Returns `null`
 * when:
 *
 *   * The verb isn't registered (no `Verb` cell entry by that name).
 *   * The verb invokes no Agent Definition.
 *   * The Agent Definition has no Model or no Prompt fact.
 *
 * Behaviour matches `arest::agent::resolve_agent_verb` exactly — same
 * cell names, same fail-closed semantics, same role-binding lookups.
 * If/when the parser changes the canonical cell-naming convention,
 * this resolver fails noisily (returns `null`); the kernel-side test
 * `resolve_against_real_agents_metamodel_cells` is the tripwire for
 * that drift.
 */
export function resolveAgentVerb(
  state: AgentVerbState,
  verbName: string,
): AgentBinding | null {
  const verbs = state.Verb ?? []
  const verb = verbs.find((v) => v.name === verbName)
  const verbId = verb?.id
  if (!verbId) return null

  const invokes = state.Verb_invokes_Agent_Definition ?? []
  const invocation = invokes.find((f) => f.Verb === verbId)
  const agentDefinitionId = invocation?.['Agent Definition']
  if (!agentDefinitionId) return null

  const models = state.Agent_Definition_uses_Model ?? []
  const modelFact = models.find((f) => f['Agent Definition'] === agentDefinitionId)
  const modelCode = modelFact?.Model
  if (!modelCode) return null

  const prompts = state.Agent_Definition_has_Prompt ?? []
  const promptFact = prompts.find((f) => f['Agent Definition'] === agentDefinitionId)
  const prompt = promptFact?.Prompt
  if (!prompt) return null

  return { modelCode, prompt, agentDefinitionId }
}

/**
 * Best-effort Model.code lookup for envelope introspection. Used on
 * the 503 path so the envelope's `agentDefinition.model` field can
 * surface a partially configured Agent Definition (e.g. Model present
 * but Prompt missing) — same introspection leak the kernel #620
 * envelope already permits via `agent::resolve_agent_verb` returning
 * `Some` for the model walk even when the resolver as a whole fails.
 *
 * Returns `null` when the verb has no AgentDef binding at all (full
 * boot-state-empty path).
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

// ── Handler ─────────────────────────────────────────────────────────

/**
 * Render the agent's prompt template with the request body as input.
 *
 * The prompt-template language is intentionally minimal in this first
 * cut — the rendered prompt is `${template}\n\nInput:\n${jsonBody}`.
 * Once the agents metamodel grows a real templating spec (placeholder
 * substitution, role-aware messages), this function moves to a
 * dedicated module. For now the LLM gets the full input as a JSON
 * blob after the agent-supplied instructions; that's enough for the
 * "extract structured fields from this email" canonical use case.
 */
function renderPrompt(template: string, body: unknown): string {
  return `${template}\n\nInput:\n${JSON.stringify(body, null, 2)}`
}

/**
 * Build the 503 envelope per #620's spec, mirrored from
 * `arest_kernel::system::build_envelope`. Same field shape so the
 * HATEOAS-aware client can branch on a single envelope schema across
 * the kernel and worker targets.
 *
 * `agentDefinition` is included when introspection finds *any* binding
 * for the verb — even partial (Model alone, or Prompt alone) — so the
 * dashboard can see what's missing without a second round trip.
 */
function buildEnvelope(
  code: string,
  message: string,
  introspection: { model?: string; prompt?: string } | null,
): Record<string, unknown> {
  const errorEntry: Record<string, unknown> = {
    code,
    message,
    verb: 'extract',
    _links: { worker: { href: EXTRACT_WORKER_URL } },
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
    retryAfter: EXTRACT_WORKER_URL,
  }
}

function envelope503(body: Record<string, unknown>): Response {
  return new Response(JSON.stringify(body), {
    status: 503,
    headers: {
      'content-type': 'application/json',
      'retry-after': EXTRACT_WORKER_URL,
    },
  })
}

/**
 * Main handler. The router calls this from the `/arest/extract` route;
 * the function is also re-exportable for the engine-side platform-fn
 * install path once that wire lands (the Cargo `cloudflare` profile
 * already exposes `register_async_platform_fn`; the worker side will
 * adapt this handler's body to the Object-shaped input/output then).
 */
export async function handleExtract(
  request: Request,
  env: AiCompleteEnv,
  opts: ExtractHandlerOptions = {},
): Promise<Response> {
  const state = opts.state ?? {}
  const aiComplete = opts.aiComplete ?? aiCompleteImpl

  // 1. Parse the request body. Malformed JSON falls into `extract.parse`
  //    so a bad POST never panics — mirror of the kernel branch.
  let body: unknown
  try {
    body = await request.json()
  } catch {
    return envelope503(
      buildEnvelope(
        'extract.parse',
        "Request body did not parse as JSON; nothing to dispatch to the 'extract' verb.",
        null,
      ),
    )
  }

  // 2. Resolve the Agent Definition. `null` → 503 with introspectable
  //    metadata (when partial config is reachable via the four-cell
  //    walk).
  const binding = resolveAgentVerb(state, 'extract')
  if (!binding) {
    return envelope503(
      buildEnvelope(
        'extract.no_body',
        "The 'extract' verb is registered on this worker but no Agent Definition is " +
          'installed for it. Seed the Agent Definition cells (Verb, ' +
          'Verb_invokes_Agent_Definition, Agent_Definition_uses_Model, ' +
          'Agent_Definition_has_Prompt) per readings/templates/agents.md, then retry.',
        introspectAgentDefinition(state, 'extract'),
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
        code: `extract.ai_complete.${result.error.code}`,
        message: result.error.message,
        verb: 'extract',
        agentDefinition: {
          model: binding.modelCode,
          // Don't echo the prompt body in the error envelope — it can
          // be huge and contain user data. The `agentDefinitionId`
          // gives the dashboard enough to fetch the full prompt via
          // `/explain` if needed.
          agentDefinitionId: binding.agentDefinitionId,
        },
        _links: { worker: { href: EXTRACT_WORKER_URL } },
      }],
      status: 503,
      retryAfter: EXTRACT_WORKER_URL,
    })
  }

  // 5. Try to parse the LLM output as JSON. The canonical extract
  //    contract is "produce a JSON object", but LLMs sometimes wander
  //    off-script (refusal text, prose preamble, hedging). On parse
  //    failure surface the raw text in `_raw` so the dashboard can
  //    inspect; the engine treats `_raw` as a structured-output miss
  //    rather than a hard failure.
  let parsed: unknown
  try {
    parsed = JSON.parse(result.text)
  } catch {
    parsed = { _raw: result.text }
  }

  return new Response(
    JSON.stringify({
      data: parsed,
      _meta: result._meta,
      _links: { self: { href: EXTRACT_WORKER_URL } },
    }),
    {
      status: 200,
      headers: { 'content-type': 'application/json' },
    },
  )
}
