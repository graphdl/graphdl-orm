/**
 * Tests for the `/arest/extract` worker handler (#639 / Worker-AI-2).
 *
 * Mirrors the kernel-side #620 dispatch contract:
 *
 *   1. Body parses JSON → resolve Agent Definition for verb "extract"
 *      → render prompt template → call `aiComplete` → return the
 *      parsed result in the HATEOAS envelope.
 *   2. No Agent Definition for "extract" in worker boot state →
 *      503 with the same `extract.no_body` envelope shape the kernel
 *      emits (introspectable Agent Definition metadata when partially
 *      configured; minimal envelope when fully absent).
 *   3. `aiComplete` returns `{ error }` → 503 (or status mapped from
 *      the error code, mirroring the /api/ai/complete handler) with
 *      the underlying `error.code` bubbled up.
 *
 * The handler does NOT throw — failures are structured envelopes that
 * map cleanly onto `Object::Bottom` at the engine boundary, same
 * contract as `aiComplete`.
 *
 * Mocks `aiComplete` directly so the test never makes a real network
 * call. The Agent Definition resolver path is mocked by passing an
 * explicit `getAgentBinding` function override; the production path
 * reads from worker boot state populated by #641.
 */

/// <reference types="node" />
import { describe, it, expect, vi, afterEach } from 'vitest'
import { handleExtract, resolveAgentVerb, type AgentVerbState } from './extract'
import type { AiCompleteResult } from './complete'

const TEST_ENV = {
  AI_GATEWAY_URL: 'https://gateway.ai.cloudflare.com/v1/test-acct/test-gw/openai',
  AI_GATEWAY_TOKEN: 'test-token-123',
}

const EXTRACT_AGENT_STATE: AgentVerbState = {
  Verb: [{ id: 'verb-extract', name: 'extract' }],
  Verb_invokes_Agent_Definition: [
    { Verb: 'verb-extract', 'Agent Definition': 'agent-extractor' },
  ],
  Agent_Definition_uses_Model: [
    { 'Agent Definition': 'agent-extractor', Model: 'gpt-4o-mini' },
  ],
  Agent_Definition_has_Prompt: [
    { 'Agent Definition': 'agent-extractor', Prompt: 'Extract structured fields from this input. Reply ONLY with JSON.' },
  ],
}

function jsonRequest(body: unknown): Request {
  return new Request('https://arest.do/arest/extract', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

function aiCompleteMockSuccess(text: string): () => Promise<AiCompleteResult> {
  return async () => ({
    text,
    _meta: { gateway: TEST_ENV.AI_GATEWAY_URL, model: 'gpt-4o-mini' },
  })
}

function aiCompleteMockError(
  code: 'config' | 'auth' | 'upstream' | 'network' | 'shape',
  message = 'mock failure',
): () => Promise<AiCompleteResult> {
  return async () => ({ error: { code, message } })
}

describe('resolveAgentVerb (TS port of arest::agent::resolve_agent_verb)', () => {
  it('returns the binding when all four cells are populated', () => {
    const binding = resolveAgentVerb(EXTRACT_AGENT_STATE, 'extract')
    expect(binding).not.toBeNull()
    expect(binding?.modelCode).toBe('gpt-4o-mini')
    expect(binding?.prompt).toContain('Extract structured fields')
    expect(binding?.agentDefinitionId).toBe('agent-extractor')
  })

  it('returns null for an unknown verb', () => {
    expect(resolveAgentVerb(EXTRACT_AGENT_STATE, 'summarise')).toBeNull()
  })

  it('returns null when the verb invokes no Agent Definition', () => {
    const state: AgentVerbState = {
      Verb: [{ id: 'verb-orphan', name: 'orphan' }],
      Verb_invokes_Agent_Definition: [],
      Agent_Definition_uses_Model: [],
      Agent_Definition_has_Prompt: [],
    }
    expect(resolveAgentVerb(state, 'orphan')).toBeNull()
  })

  it('returns null when the Agent Definition has no Model', () => {
    const state: AgentVerbState = {
      Verb: [{ id: 'verb-incomplete', name: 'incomplete' }],
      Verb_invokes_Agent_Definition: [
        { Verb: 'verb-incomplete', 'Agent Definition': 'agent-incomplete' },
      ],
      Agent_Definition_uses_Model: [],
      Agent_Definition_has_Prompt: [
        { 'Agent Definition': 'agent-incomplete', Prompt: 'p' },
      ],
    }
    expect(resolveAgentVerb(state, 'incomplete')).toBeNull()
  })

  it('returns null when the Agent Definition has no Prompt', () => {
    const state: AgentVerbState = {
      Verb: [{ id: 'verb-noprompt', name: 'noprompt' }],
      Verb_invokes_Agent_Definition: [
        { Verb: 'verb-noprompt', 'Agent Definition': 'agent-noprompt' },
      ],
      Agent_Definition_uses_Model: [
        { 'Agent Definition': 'agent-noprompt', Model: 'gpt-4o' },
      ],
      Agent_Definition_has_Prompt: [],
    }
    expect(resolveAgentVerb(state, 'noprompt')).toBeNull()
  })

  it('returns null on a fully empty state (mirrors worker boot before #641)', () => {
    expect(resolveAgentVerb({}, 'extract')).toBeNull()
  })

  it('disambiguates between multiple verbs in state', () => {
    const state: AgentVerbState = {
      Verb: [
        { id: 'verb-extract', name: 'extract' },
        { id: 'verb-chat', name: 'chat' },
      ],
      Verb_invokes_Agent_Definition: [
        { Verb: 'verb-extract', 'Agent Definition': 'agent-extractor' },
        { Verb: 'verb-chat', 'Agent Definition': 'agent-chatter' },
      ],
      Agent_Definition_uses_Model: [
        { 'Agent Definition': 'agent-extractor', Model: 'claude-sonnet-4.6' },
        { 'Agent Definition': 'agent-chatter', Model: 'gpt-4o' },
      ],
      Agent_Definition_has_Prompt: [
        { 'Agent Definition': 'agent-extractor', Prompt: 'extract.' },
        { 'Agent Definition': 'agent-chatter', Prompt: 'chat.' },
      ],
    }
    expect(resolveAgentVerb(state, 'extract')?.modelCode).toBe('claude-sonnet-4.6')
    expect(resolveAgentVerb(state, 'chat')?.modelCode).toBe('gpt-4o')
  })
})

describe('handleExtract (Worker-AI-2 / #639)', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  // ── 200 path: Agent Definition resolves + aiComplete succeeds ─────

  it('returns 200 with parsed JSON in the envelope when aiComplete returns valid JSON', async () => {
    const fixture = { name: 'Acme Corp', amount: 1200.5 }
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess(JSON.stringify(fixture)))
    const response = await handleExtract(
      jsonRequest({ text: 'invoice from Acme Corp for $1200.50' }),
      TEST_ENV,
      { state: EXTRACT_AGENT_STATE, aiComplete: aiCompleteMock },
    )

    expect(response.status).toBe(200)
    const body = await response.json() as Record<string, unknown>
    expect(body).toMatchObject({
      data: fixture,
    })
    expect(aiCompleteMock).toHaveBeenCalledOnce()
  })

  it('passes the resolved Model.code through to aiComplete', async () => {
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess('{}'))
    await handleExtract(
      jsonRequest({ text: 'hello' }),
      TEST_ENV,
      { state: EXTRACT_AGENT_STATE, aiComplete: aiCompleteMock },
    )
    const calls = aiCompleteMock.mock.calls as unknown as Array<[string, { model?: string }]>
    expect(calls[0]![1].model).toBe('gpt-4o-mini')
  })

  it('renders the prompt template with the request body as input', async () => {
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess('{}'))
    await handleExtract(
      jsonRequest({ text: 'a fact-bearing sentence' }),
      TEST_ENV,
      { state: EXTRACT_AGENT_STATE, aiComplete: aiCompleteMock },
    )
    const calls = aiCompleteMock.mock.calls as unknown as Array<[string, unknown]>
    const prompt = calls[0]![0]
    expect(prompt).toContain('Extract structured fields')
    // Body content should be threaded through so the LLM has the input
    expect(prompt).toContain('a fact-bearing sentence')
  })

  it('falls back to _raw on JSON parse failure of aiComplete output', async () => {
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess('Sorry, I cannot help with that.'))
    const response = await handleExtract(
      jsonRequest({ text: 'whatever' }),
      TEST_ENV,
      { state: EXTRACT_AGENT_STATE, aiComplete: aiCompleteMock },
    )
    expect(response.status).toBe(200)
    const body = await response.json() as { data: { _raw: string } }
    expect(body.data._raw).toBe('Sorry, I cannot help with that.')
  })

  // ── 503 path: no Agent Definition (#641 hasn't seeded yet) ─────────

  it('returns 503 with extract.no_body envelope when no Agent Definition is registered', async () => {
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess('unused'))
    const response = await handleExtract(
      jsonRequest({ text: 'whatever' }),
      TEST_ENV,
      { state: {}, aiComplete: aiCompleteMock },
    )
    expect(response.status).toBe(503)
    const body = await response.json() as {
      errors: Array<{ code: string; verb: string; _links: { worker: { href: string } } }>
      status: number
      retryAfter: string
    }
    expect(body.errors[0]!.code).toBe('extract.no_body')
    expect(body.errors[0]!.verb).toBe('extract')
    expect(body.status).toBe(503)
    expect(body.retryAfter).toContain('/arest/extract')
    // aiComplete must NOT be called when there's no binding to dispatch
    expect(aiCompleteMock).not.toHaveBeenCalled()
  })

  it('includes Retry-After header on the 503 no-body path (mirror of kernel #620)', async () => {
    const response = await handleExtract(
      jsonRequest({ text: 'whatever' }),
      TEST_ENV,
      { state: {}, aiComplete: vi.fn(aiCompleteMockSuccess('unused')) },
    )
    expect(response.status).toBe(503)
    expect(response.headers.get('retry-after')).toBeTruthy()
  })

  // ── 503 path: aiComplete failure bubble-up ──────────────────────────

  it('returns 503 envelope with bubbled-up aiComplete error code on auth failure', async () => {
    const response = await handleExtract(
      jsonRequest({ text: 'whatever' }),
      TEST_ENV,
      { state: EXTRACT_AGENT_STATE, aiComplete: aiCompleteMockError('auth', 'invalid token') },
    )
    expect(response.status).toBe(503)
    const body = await response.json() as {
      errors: Array<{ code: string; message: string; verb: string }>
    }
    expect(body.errors[0]!.code).toBe('extract.ai_complete.auth')
    expect(body.errors[0]!.message).toContain('invalid token')
    expect(body.errors[0]!.verb).toBe('extract')
  })

  it('returns 503 envelope with bubbled-up aiComplete error code on upstream failure', async () => {
    const response = await handleExtract(
      jsonRequest({ text: 'whatever' }),
      TEST_ENV,
      { state: EXTRACT_AGENT_STATE, aiComplete: aiCompleteMockError('upstream', '502 bad gateway') },
    )
    expect(response.status).toBe(503)
    const body = await response.json() as { errors: Array<{ code: string }> }
    expect(body.errors[0]!.code).toBe('extract.ai_complete.upstream')
  })

  // ── Malformed input ─────────────────────────────────────────────────

  it('returns 503 with extract.parse code on malformed JSON body (mirror of kernel #620)', async () => {
    const badRequest = new Request('https://arest.do/arest/extract', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{not json',
    })
    const response = await handleExtract(badRequest, TEST_ENV, {
      state: EXTRACT_AGENT_STATE,
      aiComplete: vi.fn(aiCompleteMockSuccess('unused')),
    })
    expect(response.status).toBe(503)
    const body = await response.json() as { errors: Array<{ code: string }> }
    expect(body.errors[0]!.code).toBe('extract.parse')
  })

  it('partial Agent Definition (model present, prompt missing) yields 503 with introspectable metadata', async () => {
    const partialState: AgentVerbState = {
      Verb: [{ id: 'verb-extract', name: 'extract' }],
      Verb_invokes_Agent_Definition: [
        { Verb: 'verb-extract', 'Agent Definition': 'agent-extractor' },
      ],
      Agent_Definition_uses_Model: [
        { 'Agent Definition': 'agent-extractor', Model: 'gpt-4o-mini' },
      ],
      // Prompt missing — resolver returns null, but introspection
      // surfaces the Model so dashboards can see the partial config.
      Agent_Definition_has_Prompt: [],
    }
    const response = await handleExtract(
      jsonRequest({ text: 'x' }),
      TEST_ENV,
      { state: partialState, aiComplete: vi.fn(aiCompleteMockSuccess('unused')) },
    )
    expect(response.status).toBe(503)
    const body = await response.json() as {
      errors: Array<{ code: string; agentDefinition?: { model?: string; prompt?: string } }>
    }
    expect(body.errors[0]!.code).toBe('extract.no_body')
    // Partial introspection surfaces the model when it's the only thing
    // configured — same shape the kernel emits.
    expect(body.errors[0]!.agentDefinition?.model).toBe('gpt-4o-mini')
  })
})
