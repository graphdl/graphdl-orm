/**
 * BASE-parameterized HTTP e2e for the AREST HATEOAS surface (#656).
 *
 * Run against the worker (default) — real production verification:
 *   pnpm test src/tests/e2e-hateoas.test.ts
 *   yarn test src/tests/e2e-hateoas.test.ts
 *
 * Run against the kernel under QEMU — local verification:
 *   BASE=http://localhost:8080 pnpm test src/tests/e2e-hateoas.test.ts
 *   BASE=http://localhost:8080 yarn test src/tests/e2e-hateoas.test.ts
 *
 * Per the contract-parity claim documented in
 * `_reports/kernel-hateoas-gap.md`: the kernel and worker MUST return
 * structurally compatible responses for these routes (#608-#618).
 *
 * The one expected divergence (#620): `/arest/extract` returns
 *   - 200 + `{ data, _meta, ... }`            on the worker once #641
 *                                              seeds the Agent Definition,
 *   - 503 + `{ errors: [{ code: 'extract.…' }] }` on the kernel until
 *                                              #641 lands seeding.
 * Both shapes are accepted here; either is "contract OK".
 *
 * Skip semantics: if `BASE` is unreachable (connection refused / DNS /
 * timeout in <2 s), the suite skips cleanly with a console message
 * instead of failing. CI-friendly: passes when BASE is reachable +
 * contract OK; skips when BASE is unreachable.
 *
 * This suite is intentionally read-mostly. The single write test (POST
 * /arest/entities/{noun}) is gated on `AREST_E2E_WRITE=1` so a
 * production worker doesn't get random fixture entities written into
 * its registry on every CI run.
 */

import { describe, it, expect, beforeAll } from 'vitest'

// E2E suite is opt-in via `BASE`. Without it, the suite skips cleanly
// rather than hitting api.auto.dev (which gates these routes behind
// auth and would surface 401s as test failures in dev).
//
// Override with `BASE=http://localhost:8080` to point at the kernel
// under QEMU, or `BASE=https://api.auto.dev` to point at the deployed
// worker. `BASE` may be either `http(s)://host` or
// `http(s)://host/prefix`; trailing slashes and `/arest` suffixes are
// normalized away so test paths can be written as `/arest/...`.
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
    // Quiet skip — the absence of BASE is the explicit opt-out. Don't
    // print a warning that would noise up every yarn-test invocation.
    return
  }
  // Probe BASE for reachability. A connection refused / DNS failure /
  // timeout flips the suite into skip mode; any HTTP response (even a
  // 404 at `/`) means BASE is up and we can run the contract tests.
  try {
    await fetchWithTimeout('/', { method: 'GET' }, 3_000)
    reach.reachable = true
  } catch (e) {
    reach.reachable = false
    reach.reason = e instanceof Error ? e.message : String(e)
    // eslint-disable-next-line no-console
    console.warn(
      `[e2e-hateoas] BASE=${BASE} unreachable (${reach.reason}); skipping suite. ` +
        `Start the worker (yarn dev) or kernel (scripts/run-e2e-against-kernel.ps1) and rerun.`,
    )
  }
})

// `it.skipIf` evaluates its argument at test-registration time, before
// `beforeAll` has run — too early for our reachability probe. So each
// test calls `skipIfUnreachable(ctx)` from inside the body, which uses
// the per-test `ctx.skip()` from vitest 4.x. Skip reason is surfaced
// once at suite level via the `beforeAll` console.warn.
function skipIfUnreachable(ctx: { skip: (note?: string) => void }): boolean {
  if (!reach.reachable) {
    ctx.skip(`BASE=${BASE} unreachable: ${reach.reason ?? 'unknown'}`)
    return true
  }
  return false
}

describe('GET /arest/parse — engine stats (#611)', () => {
  it('returns the totals/perDomain envelope', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const res = await fetchWithTimeout('/arest/parse')
    expect([200, 404, 503]).toContain(res.status)
    if (res.status !== 200) return // kernel may not have wired this yet
    const body = (await res.json()) as Record<string, unknown>
    // The contract per src/api/parse.ts handleParseGet:
    //   { totals: { domains, nouns, readings, factTypes, constraints },
    //     perDomain: Record<string, { nouns, readings }> }
    expect(body).toHaveProperty('totals')
    expect(body).toHaveProperty('perDomain')
    const totals = body.totals as Record<string, unknown>
    expect(totals).toHaveProperty('domains')
    expect(totals).toHaveProperty('nouns')
    expect(totals).toHaveProperty('readings')
  })
})

describe('GET /arest/{slug} — read fallback collection (#608-#610)', () => {
  it('returns either the envelope shape or 404 (slug unknown)', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    // organizations is the canonical seeded collection per
    // readings/templates/organizations.md.
    const res = await fetchWithTimeout('/arest/organizations')
    expect([200, 404]).toContain(res.status)
    if (res.status !== 200) return
    const body = (await res.json()) as Record<string, unknown>
    // The fallback returns either the bare collection (`{ docs, … }`)
    // or the Theorem 5 envelope (`{ data, _links, … }`). Both contracts
    // are valid — accept either.
    const hasEnvelope = 'data' in body && '_links' in body
    const hasBareList = 'docs' in body || 'totalDocs' in body
    expect(hasEnvelope || hasBareList).toBe(true)
  })
})

describe('GET /arest/{slug}/{id} — read fallback single (#608-#610)', () => {
  it('returns 200 with envelope or 404 if id unknown', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const res = await fetchWithTimeout('/arest/organizations/__e2e_probe_does_not_exist__')
    // 404 is the contract-OK answer when the id isn't seeded.
    expect([200, 404]).toContain(res.status)
    if (res.status === 200) {
      const body = (await res.json()) as Record<string, unknown>
      expect(body).toBeTruthy()
    }
  })
})

describe('POST /arest/entity — AREST command write (#614-#616)', () => {
  it('accepts an AREST command body and returns the command result envelope', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const writeMode = process.env.AREST_E2E_WRITE === '1'
    if (!writeMode) {
      // Probe with a malformed body so we hit the validation path
      // without polluting a production registry. The handler MUST
      // surface a structured error (4xx/5xx) — never 200.
      const res = await fetchWithTimeout('/arest/entity', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({}),
      })
      expect([400, 422, 404, 405, 503]).toContain(res.status)
      return
    }
    // Live write mode (gated): create an ephemeral organization.
    const res = await fetchWithTimeout('/arest/entity', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        type: 'create',
        noun: 'Organization',
        domain: 'organizations',
        data: { slug: `e2e-${Date.now()}`, name: 'e2e probe' },
      }),
    })
    expect([200, 201]).toContain(res.status)
    const body = (await res.json()) as Record<string, unknown>
    // Result envelope per src/api/router.ts /api/entity post handler.
    expect(body).toHaveProperty('id')
  })
})

describe('POST /arest/entities/{noun} — direct write (#614-#616)', () => {
  it('rejects malformed bodies with structured envelope; accepts well-formed in write mode', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const writeMode = process.env.AREST_E2E_WRITE === '1'
    if (!writeMode) {
      const res = await fetchWithTimeout('/arest/entities/Organization', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({}),
      })
      expect([400, 404, 405, 422, 503]).toContain(res.status)
      return
    }
    const res = await fetchWithTimeout('/arest/entities/Organization', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        slug: `e2e-direct-${Date.now()}`,
        name: 'e2e direct probe',
      }),
    })
    expect([200, 201]).toContain(res.status)
  })
})

describe('GET /arest/entities/{slug}/{id}/transitions — state machine (#617-#618, #643)', () => {
  it('returns transitions list or 404 if id unknown; never 5xx', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const res = await fetchWithTimeout(
      '/arest/entities/SupportRequest/__e2e_probe_does_not_exist__/transitions',
    )
    expect([200, 400, 404]).toContain(res.status)
    if (res.status === 200) {
      const body = (await res.json()) as Record<string, unknown>
      expect(body).toHaveProperty('transitions')
      expect(Array.isArray(body.transitions)).toBe(true)
    }
  })
})

describe('POST /arest/entities/{slug}/{id}/transition — fire transition (#617-#618, #643)', () => {
  it('rejects malformed transition body with structured error', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const res = await fetchWithTimeout(
      '/arest/entities/SupportRequest/__e2e_probe_does_not_exist__/transition',
      {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({}), // no `event` field
      },
    )
    expect([400, 404]).toContain(res.status)
  })
})

describe('POST /arest/extract — kernel/worker shape divergence (#620)', () => {
  it('accepts BOTH the worker 200+data shape AND the kernel 503+envelope shape', async (ctx) => {
    if (skipIfUnreachable(ctx)) return
    const res = await fetchWithTimeout('/arest/extract', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ input: 'extract structured fields from this probe' }),
    })

    // Per #620 the contract is one of:
    //   • 200 from worker once #641's Agent Definition seeds:
    //       { data: ..., _meta: { gateway, model, ... } }
    //   • 503 from kernel (or worker pre-#641):
    //       { errors: [{ code: 'extract.no_body' | 'extract.parse'
    //                          | 'extract.ai_complete.<sub>' }] }
    expect([200, 400, 503]).toContain(res.status)

    const body = (await res.json()) as Record<string, unknown>

    if (res.status === 200) {
      // Worker happy path. The handler may return either the envelope
      // (`{ data, _meta }`) or the raw extract object — both are valid
      // shapes per src/api/ai/extract.ts.
      const hasData = 'data' in body
      const hasMeta = '_meta' in body
      expect(hasData || hasMeta).toBe(true)
    } else {
      // Kernel envelope or worker pre-seed. MUST be introspectable —
      // i.e. carry an `errors` array with a `code` per error.
      expect(body).toHaveProperty('errors')
      const errors = body.errors as Array<Record<string, unknown>>
      expect(Array.isArray(errors)).toBe(true)
      expect(errors.length).toBeGreaterThan(0)
      expect(errors[0]).toHaveProperty('code')
      const code = String(errors[0].code)
      // The 'extract.*' code prefix is the cross-target invariant.
      expect(code.startsWith('extract.') || code === 'extract').toBe(true)
    }
  })
})
