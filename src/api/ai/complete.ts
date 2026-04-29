/**
 * `aiComplete` — Worker-side handler for LLM completions through the
 * Cloudflare AI Gateway (#638 / Worker-AI-1).
 *
 * Foundational primitive for the LLM-shaped external functions that
 * the engine exposes via `Func::Platform("ai_complete")`. The pattern
 * is documented in `crates/arest/src/externals.rs`:
 *
 *   - The engine treats every external call as `Func::Platform(name)`
 *     in DEFS. `apply(Func::Def(name), input, D)` runs whichever body
 *     was installed at boot. If no body is installed, the call returns
 *     `Object::Bottom` — graceful skip, never a panic.
 *   - This file is the body the Worker target installs for the name
 *     `ai_complete`. Once #639 lands the engine wire-up, every verb
 *     that walks `Verb → Agent Definition → Model → Platform` reaches
 *     this handler.
 *
 * Wire shape (canonical):
 *
 *   aiComplete(prompt, opts) → { text, _meta?, citations? }   on success
 *   aiComplete(prompt, opts) → { error: { code, message } }   on failure
 *
 * `_meta` carries enough provenance for the engine to emit a Citation
 * fact (Authority Type 'Runtime-Function'). The handler NEVER throws —
 * thrown errors would force the engine into an exceptional path; the
 * structured `{ error }` envelope maps cleanly onto `Object::Bottom` at
 * the engine boundary.
 *
 * The HTTP wire (POST /api/ai/complete) reuses this handler so callers
 * outside the worker (CLI, MCP-stdio, future kernel target) can reach
 * the same body the engine sees.
 */

// ── Env shape ───────────────────────────────────────────────────────
// Two bindings, both Cloudflare-native:
//
//   AI_GATEWAY_URL   plain var — public URL of the AI Gateway worker
//                    project. Looks like
//                    `https://gateway.ai.cloudflare.com/v1/<account>/
//                     <gateway>/<provider>` where `<provider>` is the
//                    upstream slug (`openai`, `anthropic`, `workers-ai`).
//                    The handler appends `/chat/completions` so the
//                    same URL works for any OpenAI-compatible upstream.
//
//   AI_GATEWAY_TOKEN secret — bearer token the gateway expects.
//                    For OpenAI-routed calls this is the OpenAI API
//                    key; the gateway forwards it. For Workers AI
//                    routes, it's a Cloudflare API token. Always set
//                    via `wrangler secret put AI_GATEWAY_TOKEN` in
//                    production — never commit it.
export interface AiCompleteEnv {
  readonly AI_GATEWAY_URL: string
  readonly AI_GATEWAY_TOKEN: string
}

// ── Public types ────────────────────────────────────────────────────

export interface AiCompleteOptions {
  /** Cloudflare bindings; see AiCompleteEnv. */
  readonly env: AiCompleteEnv
  /** Model slug (e.g. `gpt-4o-mini`, `claude-3-5-sonnet`, `@cf/meta/llama-3.1-8b-instruct`). */
  readonly model?: string
  /** Sampling temperature, 0–2. Defaults to provider default. */
  readonly temperature?: number
  /** Max output tokens. Defaults to provider default. */
  readonly max_tokens?: number
  /** Pass-through extras for the gateway body (response_format, tools, etc). */
  readonly extras?: Record<string, unknown>
}

export interface AiCompleteSuccess {
  readonly text: string
  /** Provenance for engine-side Citation emission. */
  readonly _meta?: {
    readonly model?: string
    readonly gateway: string
    readonly finishReason?: string
  }
  /** Reserved for tool-output / RAG citations once #640 lands. */
  readonly citations?: readonly unknown[]
}

export interface AiCompleteError {
  readonly error: {
    /** Stable error code so callers can branch without parsing the message. */
    readonly code: 'config' | 'auth' | 'upstream' | 'network' | 'shape'
    readonly message: string
    readonly status?: number
  }
}

export type AiCompleteResult = AiCompleteSuccess | AiCompleteError

// ── Implementation ──────────────────────────────────────────────────

/**
 * Issue a chat completion against the configured AI Gateway. Returns a
 * structured success or error envelope — never throws. Safe to install
 * via `register_async_platform_fn("ai_complete", …)` once #639 wires
 * the engine call site.
 */
export async function aiComplete(
  prompt: string,
  opts: AiCompleteOptions,
): Promise<AiCompleteResult> {
  const { env, model, temperature, max_tokens, extras } = opts

  if (!env.AI_GATEWAY_URL || !env.AI_GATEWAY_TOKEN) {
    return {
      error: {
        code: 'config',
        message:
          'AI_GATEWAY_URL and AI_GATEWAY_TOKEN must be set on the worker. ' +
          'Run `wrangler secret put AI_GATEWAY_TOKEN` and configure ' +
          'AI_GATEWAY_URL in wrangler.jsonc.',
      },
    }
  }

  const url = joinUrl(env.AI_GATEWAY_URL, '/chat/completions')

  // Canonical OpenAI-compatible chat envelope. The Cloudflare AI Gateway
  // accepts this shape for the `openai` and `anthropic` upstreams (it
  // translates internally for Workers AI). One-message conversation —
  // multi-turn history can be threaded later via `extras`.
  const body: Record<string, unknown> = {
    messages: [{ role: 'user', content: prompt }],
    ...(model !== undefined && { model }),
    ...(temperature !== undefined && { temperature }),
    ...(max_tokens !== undefined && { max_tokens }),
    ...(extras ?? {}),
  }

  let response: Response
  try {
    response = await fetch(url, {
      method: 'POST',
      headers: {
        'authorization': `Bearer ${env.AI_GATEWAY_TOKEN}`,
        'content-type': 'application/json',
      },
      body: JSON.stringify(body),
    })
  } catch (e) {
    return {
      error: {
        code: 'network',
        message: e instanceof Error ? e.message : String(e),
      },
    }
  }

  if (!response.ok) {
    // 401/403 → auth; everything else upstream. Keeping the codes
    // small lets engine-side dispatch branch on `code` without a
    // schema lookup.
    const status = response.status
    const code: AiCompleteError['error']['code'] =
      status === 401 || status === 403 ? 'auth' : 'upstream'
    let detail = `${status} ${response.statusText}`
    try {
      const body = (await response.json()) as { error?: { message?: string } | string }
      const msg = typeof body.error === 'string' ? body.error : body.error?.message
      if (msg) detail += `: ${msg}`
    } catch {
      try {
        const text = await response.text()
        if (text) detail += `: ${text.slice(0, 200)}`
      } catch { /* keep status-only detail */ }
    }
    return { error: { code, message: detail, status } }
  }

  let data: ChatCompletionResponse
  try {
    data = (await response.json()) as ChatCompletionResponse
  } catch (e) {
    return {
      error: {
        code: 'shape',
        message: `gateway returned non-JSON body: ${e instanceof Error ? e.message : String(e)}`,
      },
    }
  }

  const choice = data.choices?.[0]
  const text = choice?.message?.content
  if (!choice || typeof text !== 'string') {
    return {
      error: {
        code: 'shape',
        message: 'gateway response missing choices[0].message.content',
      },
    }
  }

  return {
    text,
    _meta: {
      model: data.model ?? model,
      gateway: env.AI_GATEWAY_URL,
      finishReason: choice.finish_reason,
    },
  }
}

// ── Internals ───────────────────────────────────────────────────────

interface ChatCompletionResponse {
  readonly id?: string
  readonly model?: string
  readonly choices?: ReadonlyArray<{
    readonly index?: number
    readonly message?: { readonly role?: string; readonly content?: string }
    readonly finish_reason?: string
  }>
}

/** Concatenate base + path with exactly one slash, preserving query strings. */
function joinUrl(base: string, path: string): string {
  const trimmedBase = base.replace(/\/+$/, '')
  const trimmedPath = path.startsWith('/') ? path : `/${path}`
  return `${trimmedBase}${trimmedPath}`
}
