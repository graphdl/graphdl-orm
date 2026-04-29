/**
 * Tests for `aiComplete` — the worker-side handler that proxies LLM
 * completions through the Cloudflare AI Gateway (#638 / Worker-AI-1).
 *
 * Mocks the global `fetch` so the test never makes a real network
 * call. Asserts the handler's wire shape:
 *
 *   - Success path returns `{ text: string }` (with optional
 *     `citations` and `_meta` for provenance).
 *   - Failure paths (auth, network, malformed body) return
 *     `{ error: { code, message } }` — never throw. The engine
 *     treats `Object::Bottom` as "failed external call"; the
 *     handler emits a structured error envelope for that mapping.
 *
 * The handler reads two env-shaped values: `AI_GATEWAY_URL` and
 * `AI_GATEWAY_TOKEN`. `AI_GATEWAY_TOKEN` is a Cloudflare secret in
 * production; in tests we pass plain strings.
 */

/// <reference types="node" />
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { aiComplete, type AiCompleteEnv, type AiCompleteResult } from './complete'

const TEST_ENV: AiCompleteEnv = {
  AI_GATEWAY_URL: 'https://gateway.ai.cloudflare.com/v1/test-acct/test-gw/openai',
  AI_GATEWAY_TOKEN: 'test-token-123',
}

function jsonResponse(body: unknown, ok = true, status = 200): Response {
  return {
    ok,
    status,
    statusText: ok ? 'OK' : 'Error',
    headers: new Headers({ 'content-type': 'application/json' }),
    json: async () => body,
    text: async () => JSON.stringify(body),
  } as Response
}

describe('aiComplete (Worker-AI-1 / #638)', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  // ── Happy path ──────────────────────────────────────────────────────

  it('returns { text } on a 200 OpenAI-shaped completion', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse({
        id: 'chatcmpl-1',
        object: 'chat.completion',
        choices: [{
          index: 0,
          message: { role: 'assistant', content: 'Hello, world.' },
          finish_reason: 'stop',
        }],
      }),
    ))
    const result = await aiComplete('say hello', { env: TEST_ENV })
    expect('error' in result).toBe(false)
    expect((result as { text: string }).text).toBe('Hello, world.')
  })

  it('passes prompt as a user message to the gateway', async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ choices: [{ message: { content: 'ok' } }] }),
    )
    vi.stubGlobal('fetch', fetchMock)
    await aiComplete('extract entities from this text', { env: TEST_ENV })

    expect(fetchMock).toHaveBeenCalledOnce()
    const [_url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    const body = JSON.parse(init.body as string)
    expect(body.messages).toEqual([
      { role: 'user', content: 'extract entities from this text' },
    ])
  })

  it('threads model + temperature + max_tokens through to the gateway', async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ choices: [{ message: { content: 'ok' } }] }),
    )
    vi.stubGlobal('fetch', fetchMock)
    await aiComplete('hi', {
      env: TEST_ENV,
      model: 'gpt-4o-mini',
      temperature: 0.2,
      max_tokens: 256,
    })

    const [_url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    const body = JSON.parse(init.body as string)
    expect(body.model).toBe('gpt-4o-mini')
    expect(body.temperature).toBe(0.2)
    expect(body.max_tokens).toBe(256)
  })

  it('sends Authorization: Bearer <AI_GATEWAY_TOKEN>', async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ choices: [{ message: { content: 'ok' } }] }),
    )
    vi.stubGlobal('fetch', fetchMock)
    await aiComplete('hi', { env: TEST_ENV })

    const [_url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    const headers = new Headers(init.headers)
    expect(headers.get('authorization')).toBe('Bearer test-token-123')
    expect(headers.get('content-type')).toBe('application/json')
  })

  it('POSTs to AI_GATEWAY_URL with /chat/completions appended', async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ choices: [{ message: { content: 'ok' } }] }),
    )
    vi.stubGlobal('fetch', fetchMock)
    await aiComplete('hi', { env: TEST_ENV })

    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    expect(init.method).toBe('POST')
    expect(url).toContain(TEST_ENV.AI_GATEWAY_URL)
    expect(url).toMatch(/\/chat\/completions$/)
  })

  // ── Failure envelopes ───────────────────────────────────────────────

  it('returns { error } on 401 auth failure (no throw)', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse({ error: { message: 'invalid token' } }, false, 401),
    ))
    const result = await aiComplete('hi', { env: TEST_ENV })
    expect('error' in result).toBe(true)
    const err = (result as { error: { code: string; message: string } }).error
    expect(err.code).toBe('auth')
    expect(err.message).toMatch(/401|invalid|unauthor/i)
  })

  it('returns { error } on 5xx gateway failure', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse({ error: 'upstream timeout' }, false, 503),
    ))
    const result = await aiComplete('hi', { env: TEST_ENV })
    expect('error' in result).toBe(true)
    const err = (result as { error: { code: string; message: string } }).error
    expect(err.code).toBe('upstream')
  })

  it('returns { error } when fetch itself throws (network failure)', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => {
      throw new Error('network unreachable')
    }))
    const result = await aiComplete('hi', { env: TEST_ENV })
    expect('error' in result).toBe(true)
    const err = (result as { error: { code: string; message: string } }).error
    expect(err.code).toBe('network')
    expect(err.message).toContain('network unreachable')
  })

  it('returns { error } when env binding is missing', async () => {
    const result = await aiComplete('hi', {
      env: { AI_GATEWAY_URL: '', AI_GATEWAY_TOKEN: '' },
    })
    expect('error' in result).toBe(true)
    const err = (result as { error: { code: string; message: string } }).error
    expect(err.code).toBe('config')
  })

  it('returns { error } on malformed gateway response (no choices)', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse({ id: 'chatcmpl-1' }),
    ))
    const result = await aiComplete('hi', { env: TEST_ENV })
    expect('error' in result).toBe(true)
    const err = (result as { error: { code: string; message: string } }).error
    expect(err.code).toBe('shape')
  })

  // ── Provenance ──────────────────────────────────────────────────────
  // The Worker-side handler returns enough metadata for the engine to
  // emit a Citation fact (Authority Type 'Runtime-Function', per
  // crates/arest/src/externals.rs). We surface it in `_meta` so the
  // caller can wrap it into the engine's citation envelope.

  it('includes _meta.model + _meta.gateway in the success envelope', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse({ choices: [{ message: { content: 'ok' } }], model: 'gpt-4o-mini' }),
    ))
    const result = await aiComplete('hi', { env: TEST_ENV, model: 'gpt-4o-mini' })
    expect('error' in result).toBe(false)
    const success = result as Extract<AiCompleteResult, { text: string }>
    expect(success._meta?.model).toBe('gpt-4o-mini')
    expect(success._meta?.gateway).toContain('gateway.ai.cloudflare.com')
  })
})
