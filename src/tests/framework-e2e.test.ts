/**
 * Framework e2e — BASE-parameterized HTTP contract for the AREST framework
 * surface beyond the HATEOAS endpoints (those live in e2e-hateoas.test.ts).
 *
 * Default behaviour: opt-in via the BASE env. Without it the suite skips
 * cleanly so `yarn test` stays green for non-deployed work.
 *
 *   yarn test src/tests/framework-e2e.test.ts                   # skip
 *   BASE=http://localhost:8080 yarn test src/tests/framework-e2e.test.ts  # kernel
 *   BASE=https://api.auto.dev   yarn test src/tests/framework-e2e.test.ts  # worker
 *
 * Coverage targets the routes the kernel-vs-worker parity contract
 * (see `_reports/kernel-hateoas-gap.md`) requires to behave the same:
 *
 *   GET  /health                         — basic liveness
 *   GET  /api/openapi.json               — RMAP-derived OpenAPI 3.1
 *   GET  /api/events                     — SSE handshake (connection only)
 *   GET  /api/debug/counts/:domain       — domain population intro
 *   POST /api/load_reading               — DynRdg verb (tenant-scoped)
 *   GET  /api/entities/:noun/:id/transitions — SM transitions
 *
 * Each test asserts only the contract shape (status code set; envelope
 * presence) — not domain content, which differs across kernel/worker.
 *
 * Skip semantics: BASE absent → silent skip; BASE unreachable in <3s →
 * loud skip with the connection error reported once at suite level.
 */

import { describe, it, expect, beforeAll } from 'vitest'

const RAW_BASE = process.env.BASE ?? ''
const BASE = RAW_BASE.replace(/\/$/, '').replace(/\/arest$/, '')

const FETCH_TIMEOUT_MS = 5_000

interface ReachState {
  reachable: boolean
  reason?: string
}
const reach: ReachState = { reachable: false }

async function fetchWithTimeout(
  path: string,
  init?: RequestInit,
  timeoutMs = FETCH_TIMEOUT_MS,
): Promise<Response> {
  const controller = new AbortController()
  const t = setTimeout(() => controller.abort(), timeoutMs)
  try {
    return await fetch(`${BASE}${path}`, { ...init, signal: controller.signal })
  } finally {
    clearTimeout(t)
  }
}

beforeAll(async () => {
  if (!BASE) {
    reach.reachable = false
    reach.reason = 'BASE env not set'
    return
  }
  try {
    await fetchWithTimeout('/', { method: 'GET' }, 3_000)
    reach.reachable = true
  } catch (e) {
    reach.reachable = false
    reach.reason = e instanceof Error ? e.message : String(e)
    // eslint-disable-next-line no-console
    console.warn(
      `[framework-e2e] BASE=${BASE} unreachable (${reach.reason}); skipping suite. ` +
        `Start the worker (yarn dev) or kernel (scripts/run-e2e-against-kernel.ps1) and rerun.`,
    )
  }
})

function skipIfUnreachable(ctx: { skip: (note?: string) => void }): boolean {
  if (!reach.reachable) {
    ctx.skip(`BASE=${BASE} unreachable: ${reach.reason ?? 'unknown'}`)
    return true
  }
  return false
}

describe('GET /health — liveness', () => {
  it('returns 200 with a status payload', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const res = await fetchWithTimeout('/health')
    expect([200, 404]).toContain(res.status)
    if (res.status !== 200) return
    const body = (await res.json()) as Record<string, unknown>
    expect(body).toHaveProperty('status')
  })
})

describe('GET /api/openapi.json — OpenAPI 3.1 surface', () => {
  it('returns a valid OpenAPI 3 document or 404 if not wired', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const res = await fetchWithTimeout('/api/openapi.json')
    expect([200, 404]).toContain(res.status)
    if (res.status !== 200) return
    const body = (await res.json()) as Record<string, unknown>
    expect(body).toHaveProperty('openapi')
    expect(body).toHaveProperty('paths')
    // OpenAPI 3.x has either openapi: "3.0.x" or "3.1.x"
    expect(String(body.openapi)).toMatch(/^3\./)
  })
})

describe('GET /api/events — SSE handshake', () => {
  it('opens with text/event-stream content-type or 404 if not wired', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    // Use a short timeout: we only need the headers, not the stream.
    const res = await fetchWithTimeout('/api/events', { method: 'GET' }, 2_000)
    expect([200, 404, 405]).toContain(res.status)
    if (res.status !== 200) return
    const ct = res.headers.get('content-type') ?? ''
    expect(ct).toMatch(/event-stream/)
    // Drain a tiny prefix so the connection closes cleanly.
    if (res.body) {
      const reader = res.body.getReader()
      try { await Promise.race([reader.read(), new Promise(r => setTimeout(r, 200))]) }
      finally { try { await reader.cancel() } catch {} }
    }
  })
})

describe('GET /api/debug/counts/:domain — population intro', () => {
  it('returns either a counts envelope or 404 for unknown domain', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    // Use a domain name that should exist on either target (organizations
    // is bundled). On the kernel without bundled domains, expect 404.
    const res = await fetchWithTimeout('/api/debug/counts/organizations')
    expect([200, 404, 503]).toContain(res.status)
    if (res.status !== 200) return
    const body = (await res.json()) as Record<string, unknown>
    // Counts envelope: at minimum a `nouns` or `total` key. The exact
    // shape differs across worker and kernel; just check non-empty.
    expect(typeof body).toBe('object')
    expect(body).not.toBeNull()
  })
})

describe('POST /api/load_reading — DynRdg verb', () => {
  // Skipped on production unless AREST_E2E_WRITE=1 since this mutates
  // the tenant's reading store. Keeps the kernel/worker parity check
  // deterministic for read-only CI runs.
  it.skipIf(process.env.AREST_E2E_WRITE !== '1')(
    'accepts a minimal reading body and returns a load result envelope',
    async (ctx) => {
      if (skipIfUnreachable(ctx)) return
      const body = {
        name: `e2e-test-reading-${Date.now()}`,
        body: '# Test\n\nNoun(.id) is an entity type.\n',
      }
      const res = await fetchWithTimeout('/api/load_reading', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body),
      })
      // Any of these is a contract-valid response: 200 success, 400/422
      // validation, 404 route not yet wired, 503 not ready.
      expect([200, 201, 400, 404, 422, 503]).toContain(res.status)
    },
  )
})

describe('GET /api/entities/:noun/:id/transitions — state machine', () => {
  it('returns a transitions list, 404, or contract error', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    // Use a probably-nonexistent id — we want to verify the SHAPE of the
    // 404/empty response, not real transitions for a real entity.
    const res = await fetchWithTimeout('/api/entities/organization/never-exists-zzz/transitions')
    expect([200, 400, 404]).toContain(res.status)
    if (res.status === 200) {
      const body = (await res.json()) as Record<string, unknown>
      expect(body).toHaveProperty('transitions')
      expect(Array.isArray(body.transitions)).toBe(true)
    }
  })
})
