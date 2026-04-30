/**
 * BASE-parameterized HTTP e2e for the AREST AI-dispatch surface
 * (#642 / Worker-AI-5).
 *
 * Runs the same shape of contract probes that
 * `src/tests/e2e-hateoas.test.ts` uses for the HATEOAS reads + writes,
 * narrowed to the two AI verbs that #639 (extract) + #640 (chat)
 * migrated to the engine-style `Func::Def` dispatch chain. Once #641
 * landed the boot-time Agent Definition seed, the worker should:
 *
 *   • return 200 + a parsed envelope when AI_GATEWAY_URL +
 *     AI_GATEWAY_TOKEN are configured (real LLM round-trip), OR
 *   • return 503 + the documented `{ errors: [{ code: 'extract.…' |
 *     'chat.…', agentDefinition: { agentDefinitionId, model } }] }`
 *     envelope when the env vars are absent — proves Agent Definition
 *     resolution worked end-to-end, only the upstream LLM call failed.
 *
 * Both shapes are accepted here. The 503 path is the cross-target
 * contract guarantee (#620 + #639 + #640): the kernel and the worker
 * emit a structurally identical envelope so HATEOAS-aware clients can
 * branch on a single error schema regardless of which target serves
 * the request.
 *
 * Run against the deployed worker (default):
 *   yarn test src/tests/e2e-ai.test.ts
 *
 * Run against `wrangler dev` locally (no real API keys needed — the
 * 503 path is what we exercise without AI_GATEWAY env vars):
 *   BASE=http://127.0.0.1:8787 yarn test src/tests/e2e-ai.test.ts
 *
 * Run via the harness script that boots wrangler dev for you and
 * tears it down at exit:
 *   .\scripts\run-e2e-against-worker.ps1
 *
 * Skip semantics: if `BASE` is unreachable (connection refused / DNS /
 * timeout in <3 s on the `/health` probe), the suite skips cleanly with
 * a console message instead of failing — same skip-if-unreachable
 * pattern e2e-hateoas.test.ts (#656) already uses, so CI runs that
 * happen before deploy / before wrangler-dev is up don't go red.
 */

import { describe, it, expect, beforeAll } from 'vitest'
import {
  EXTRACTOR_AGENT_ID,
  CHATTER_AGENT_ID,
} from '../api/ai/agent-seed'

// Default BASE points at the worker production URL. Override to
// `http://127.0.0.1:8787` to run against `wrangler dev`, or
// `http://localhost:8080` to run against the kernel under QEMU
// (the kernel has the same envelope contract per #620).
//
// Trailing slashes and `/arest` suffixes are normalized away so test
// paths can be written as `/arest/...`.
const RAW_BASE = process.env.BASE ?? 'https://api.auto.dev'
const BASE = RAW_BASE.replace(/\/$/, '').replace(/\/arest$/, '')

// Optional bearer token for deployed workers that gate requests behind
// `API_SECRET` per src/worker.ts. Local `wrangler dev` instances without
// API_SECRET set ignore this and accept all requests. Pass via:
//   $env:AREST_API_TOKEN = "<secret>"; yarn test src/tests/e2e-ai.test.ts
const API_TOKEN = process.env.AREST_API_TOKEN ?? ''

const FETCH_TIMEOUT_MS = 8_000 // longer than HATEOAS — LLM round-trip can be slow on cold start

interface ReachState {
  reachable: boolean
  reason?: string
  /** True when the BASE responded but every request gates behind 401 — suite skips. */
  unauthenticated: boolean
}

const reach: ReachState = { reachable: false, unauthenticated: false }

function authHeaders(): Record<string, string> {
  return API_TOKEN ? { authorization: `Bearer ${API_TOKEN}` } : {}
}

async function fetchWithTimeout(
  path: string,
  init?: RequestInit,
  timeoutMs = FETCH_TIMEOUT_MS,
): Promise<Response> {
  const controller = new AbortController()
  const t = setTimeout(() => controller.abort(), timeoutMs)
  try {
    return await fetch(`${BASE}${path}`, {
      ...init,
      headers: { ...authHeaders(), ...(init?.headers ?? {}) },
      signal: controller.signal,
    })
  } finally {
    clearTimeout(t)
  }
}

beforeAll(async () => {
  // Probe BASE for reachability. A connection refused / DNS failure /
  // timeout flips the suite into skip mode; any HTTP response means
  // BASE is up.
  //
  // Two probes:
  //   1. `/` (no auth) — confirms the network is reachable.
  //   2. `/arest/extract` POST (with auth headers if AREST_API_TOKEN is
  //      set) — confirms the auth middleware lets us through. A 401
  //      here means the deployed worker has API_SECRET set but we
  //      didn't provide AREST_API_TOKEN; we skip the suite cleanly with
  //      a console.warn so CI runs that don't have the secret don't
  //      go red.
  try {
    await fetchWithTimeout('/', { method: 'GET' }, 3_000)
    reach.reachable = true
  } catch (e) {
    reach.reason = e instanceof Error ? e.message : String(e)
    // eslint-disable-next-line no-console
    console.warn(
      `[e2e-ai] BASE=${BASE} unreachable (${reach.reason}); skipping suite. ` +
        `Start the worker (yarn dev) or set BASE to a deployed worker URL and rerun.`,
    )
    return
  }

  // Auth probe: try a real /arest/extract POST so a deployed worker
  // without our token (or with a stale one) skips rather than failing.
  try {
    const probe = await fetchWithTimeout(
      '/arest/extract',
      {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ probe: '_e2e_auth_check' }),
      },
      3_000,
    )
    if (probe.status === 401 || probe.status === 403) {
      reach.unauthenticated = true
      // eslint-disable-next-line no-console
      console.warn(
        `[e2e-ai] BASE=${BASE} returned ${probe.status} on /arest/extract — auth required. ` +
          `Set AREST_API_TOKEN to the deployed API_SECRET (wrangler secret get) ` +
          `or run against \`wrangler dev\` locally where auth is disabled. Skipping suite.`,
      )
    }
  } catch {
    // Probe failed mid-flight — let the per-test fetches surface the
    // real error rather than blanket-skipping.
  }
})

// `it.skipIf` evaluates its argument at test-registration time, before
// `beforeAll` has run — too early for our reachability probe. So each
// test calls `skipIfUnreachable(ctx)` from inside the body. Skip reason
// is surfaced once at suite level via the `beforeAll` console.warn.
function skipIfUnreachable(ctx: { skip: (note?: string) => void }): boolean {
  if (!reach.reachable) {
    ctx.skip(`BASE=${BASE} unreachable: ${reach.reason ?? 'unknown'}`)
    return true
  }
  if (reach.unauthenticated) {
    ctx.skip(
      `BASE=${BASE} requires bearer auth; set AREST_API_TOKEN env var. ` +
        `See beforeAll console.warn for details.`,
    )
    return true
  }
  return false
}

// Shared envelope-shape assertion. Both verbs emit the same envelope
// structure on the 503 path (#620 / #639 / #640), so the assertion
// is verb-parameterized and reused for both /arest/extract and
// /arest/chat.
//
// The `expectedAgentId` argument lets each verb's test pin the
// resolved Agent Definition id when the seed populated state — proves
// the four-cell walker reached the right binding.
function assertVerbEnvelope(
  body: Record<string, unknown>,
  verbPrefix: 'extract' | 'chat',
  expectedAgentId: string,
): void {
  expect(body).toHaveProperty('errors')
  const errors = body.errors as Array<Record<string, unknown>>
  expect(Array.isArray(errors)).toBe(true)
  expect(errors.length).toBeGreaterThan(0)
  const first = errors[0]!
  expect(first).toHaveProperty('code')
  const code = String(first.code)
  // Cross-target invariant: every failure code starts with the verb
  // prefix. Acceptable shapes:
  //   `<verb>.no_body` / `<verb>.parse` / `<verb>.ai_complete.<sub>`
  expect(code.startsWith(`${verbPrefix}.`)).toBe(true)
  // The `verb` field on the error mirrors the route — handy for clients
  // that aggregate errors across verbs.
  if ('verb' in first) {
    expect(first.verb).toBe(verbPrefix)
  }
  // When the resolver walked all four cells (the 200-but-LLM-failed
  // path), the envelope carries the resolved agentDefinitionId. When
  // the boot state was empty (pre-#641 fallback), there's no
  // agentDefinition block at all — only the no_body envelope. Either
  // path is contract-OK; we only assert the id matches when present.
  const agentDef = first.agentDefinition as
    | { agentDefinitionId?: string; model?: string }
    | undefined
  if (agentDef?.agentDefinitionId !== undefined) {
    expect(agentDef.agentDefinitionId).toBe(expectedAgentId)
  }
}

describe('POST /arest/extract — Agent Definition dispatch (#642 / Worker-AI-5)', () => {
  it('returns 200 with a parsed envelope OR 503 with the extract.* error envelope', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const res = await fetchWithTimeout('/arest/extract', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        text:
          'Invoice from Acme Corp for $1,200.50 dated 2026-04-30, ' +
          'reference INV-9001.',
      }),
    })
    // Acceptable response codes:
    //   200 — Agent Definition resolved + AI_GATEWAY env reachable + LLM responded.
    //   400 — body parse failure (shouldn't happen here; we send valid JSON).
    //   503 — Agent Definition resolved but AI_GATEWAY env empty/upstream down,
    //         OR Agent Definition not seeded (pre-#641 fallback, accepted for
    //         compatibility with kernel deploys that haven't run init yet).
    expect([200, 400, 503]).toContain(res.status)

    const body = (await res.json()) as Record<string, unknown>

    if (res.status === 200) {
      // Worker happy path: { data: <parsed JSON | { _raw }>, _meta, _links }
      // per src/api/ai/extract.ts handleExtract success branch.
      expect(body).toHaveProperty('data')
      // _meta carries provenance for engine-side Citation emission.
      // Optional in the contract (handler passes through whatever
      // aiComplete returned), but always present on the worker path.
    } else if (res.status === 503) {
      assertVerbEnvelope(body, 'extract', EXTRACTOR_AGENT_ID)
    }
  })

  it('returns 503 with extract.parse on a malformed JSON body', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const res = await fetchWithTimeout('/arest/extract', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: 'not json at all',
    })
    // Some Cloudflare frontends reject non-JSON before the handler sees
    // it (400). The handler's own contract is 503 + extract.parse.
    expect([400, 503]).toContain(res.status)
    if (res.status === 503) {
      const body = (await res.json()) as Record<string, unknown>
      expect(body).toHaveProperty('errors')
      const errors = body.errors as Array<Record<string, unknown>>
      expect(String(errors[0]!.code)).toMatch(/^extract\.parse$/)
    }
  })
})

describe('POST /arest/chat — Agent Definition dispatch (#642 / Worker-AI-5)', () => {
  it('returns 200 with a text passthrough OR 503 with the chat.* error envelope', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const res = await fetchWithTimeout('/arest/chat', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        message: 'Hello — what is 2 + 2? Reply with just the number.',
      }),
    })
    expect([200, 400, 503]).toContain(res.status)

    const body = (await res.json()) as Record<string, unknown>

    if (res.status === 200) {
      // Worker happy path: { text, _meta?, citations?, _links } per
      // src/api/ai/chat.ts handleChat success branch.
      expect(body).toHaveProperty('text')
      expect(typeof body.text).toBe('string')
    } else if (res.status === 503) {
      assertVerbEnvelope(body, 'chat', CHATTER_AGENT_ID)
    }
  })

  it('returns 503 with chat.parse on a malformed JSON body', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const res = await fetchWithTimeout('/arest/chat', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: 'not json at all',
    })
    expect([400, 503]).toContain(res.status)
    if (res.status === 503) {
      const body = (await res.json()) as Record<string, unknown>
      expect(body).toHaveProperty('errors')
      const errors = body.errors as Array<Record<string, unknown>>
      expect(String(errors[0]!.code)).toMatch(/^chat\.parse$/)
    }
  })
})
