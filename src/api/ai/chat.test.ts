/**
 * Tests for the `/arest/chat` worker handler (#640 / Worker-AI-3).
 *
 * Mirrors the `extract.test.ts` shape (#639) since `chat.ts` follows
 * the same dispatch-through-engine `Func::Def` chain. Three distinct
 * branches are covered:
 *
 *   1. Body parses JSON → resolve Agent Definition for verb "chat"
 *      → render prompt template → call `aiComplete` → 200 with
 *      `{ text, _meta?, citations? }`. Unlike `extract`, no JSON
 *      parse of the LLM output — chat output is natural prose.
 *   2. No Agent Definition for "chat" in worker boot state → 503
 *      with the `chat.no_body` envelope. Mirror of `extract.no_body`
 *      so HATEOAS-aware clients can branch on a single envelope
 *      schema across both verbs.
 *   3. `aiComplete` returns `{ error }` → 503 with the underlying
 *      `error.code` bubbled up as `chat.ai_complete.<code>`.
 *
 * The handler does NOT throw — failures are structured envelopes that
 * map cleanly onto `Object::Bottom` at the engine boundary, same
 * contract as `aiComplete` and the `/arest/extract` handler.
 */

/// <reference types="node" />
import { describe, it, expect, vi, afterEach } from 'vitest'
import { handleChat, CHAT_WORKER_URL } from './chat'
import type { AgentVerbState } from './extract'
import type { AiCompleteResult } from './complete'

const TEST_ENV = {
  AI_GATEWAY_URL: 'https://gateway.ai.cloudflare.com/v1/test-acct/test-gw/openai',
  AI_GATEWAY_TOKEN: 'test-token-123',
}

const CHAT_AGENT_STATE: AgentVerbState = {
  Verb: [{ id: 'verb-chat', name: 'chat' }],
  Verb_invokes_Agent_Definition: [
    { Verb: 'verb-chat', 'Agent Definition': 'agent-chatter' },
  ],
  Agent_Definition_uses_Model: [
    { 'Agent Definition': 'agent-chatter', Model: 'gpt-4o-mini' },
  ],
  Agent_Definition_has_Prompt: [
    { 'Agent Definition': 'agent-chatter', Prompt: 'You are a helpful conversational assistant.' },
  ],
}

function jsonRequest(body: unknown): Request {
  return new Request('https://arest.do/arest/chat', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

function aiCompleteMockSuccess(
  text: string,
  citations?: readonly unknown[],
): () => Promise<AiCompleteResult> {
  return async () => ({
    text,
    _meta: { gateway: TEST_ENV.AI_GATEWAY_URL, model: 'gpt-4o-mini' },
    ...(citations !== undefined && { citations }),
  })
}

function aiCompleteMockError(
  code: 'config' | 'auth' | 'upstream' | 'network' | 'shape',
  message = 'mock failure',
): () => Promise<AiCompleteResult> {
  return async () => ({ error: { code, message } })
}

describe('handleChat (Worker-AI-3 / #640)', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  // ── 200 path: Agent Definition resolves + aiComplete succeeds ─────

  it('returns 200 with the LLM text in the envelope on a successful chat', async () => {
    const reply = 'Hello! How can I help you today?'
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess(reply))
    const response = await handleChat(
      jsonRequest({ message: 'hi' }),
      TEST_ENV,
      { state: CHAT_AGENT_STATE, aiComplete: aiCompleteMock },
    )

    expect(response.status).toBe(200)
    const body = await response.json() as Record<string, unknown>
    expect(body).toMatchObject({ text: reply })
    expect(aiCompleteMock).toHaveBeenCalledOnce()
  })

  it('surfaces _meta from aiComplete in the 200 envelope', async () => {
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess('ok'))
    const response = await handleChat(
      jsonRequest({ message: 'hi' }),
      TEST_ENV,
      { state: CHAT_AGENT_STATE, aiComplete: aiCompleteMock },
    )
    expect(response.status).toBe(200)
    const body = await response.json() as { _meta?: { model?: string } }
    expect(body._meta?.model).toBe('gpt-4o-mini')
  })

  it('passes the resolved Model.code through to aiComplete', async () => {
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess('ok'))
    await handleChat(
      jsonRequest({ message: 'hi' }),
      TEST_ENV,
      { state: CHAT_AGENT_STATE, aiComplete: aiCompleteMock },
    )
    const calls = aiCompleteMock.mock.calls as unknown as Array<[string, { model?: string }]>
    expect(calls[0]![1].model).toBe('gpt-4o-mini')
  })

  it('renders the prompt template with the request body as input', async () => {
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess('ok'))
    await handleChat(
      jsonRequest({ message: 'tell me about FORML 2' }),
      TEST_ENV,
      { state: CHAT_AGENT_STATE, aiComplete: aiCompleteMock },
    )
    const calls = aiCompleteMock.mock.calls as unknown as Array<[string, unknown]>
    const prompt = calls[0]![0]
    expect(prompt).toContain('helpful conversational assistant')
    // Body content threaded through so the LLM has the user's input
    expect(prompt).toContain('tell me about FORML 2')
  })

  it('passes through citations when aiComplete provides them', async () => {
    const cites: readonly unknown[] = [{ source: 'paper-1', span: [10, 42] }]
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess('cited reply', cites))
    const response = await handleChat(
      jsonRequest({ message: 'q' }),
      TEST_ENV,
      { state: CHAT_AGENT_STATE, aiComplete: aiCompleteMock },
    )
    expect(response.status).toBe(200)
    const body = await response.json() as { citations?: readonly unknown[] }
    expect(body.citations).toEqual(cites)
  })

  it('does NOT JSON-parse the LLM output (chat is natural prose, unlike extract)', async () => {
    // If the handler accidentally JSON-parsed natural-language text,
    // the 200 path would fall back to a `_raw` field like extract.ts.
    // For chat, the contract is "text passes through verbatim".
    const reply = 'That is a great question. Here is what I think.'
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess(reply))
    const response = await handleChat(
      jsonRequest({ message: 'q' }),
      TEST_ENV,
      { state: CHAT_AGENT_STATE, aiComplete: aiCompleteMock },
    )
    const body = await response.json() as Record<string, unknown>
    expect(body.text).toBe(reply)
    expect(body).not.toHaveProperty('data')
    expect(body).not.toHaveProperty('_raw')
  })

  // ── 503 path: no Agent Definition (#641 hasn't seeded yet) ─────────

  it('returns 503 with chat.no_body envelope when no Agent Definition is registered', async () => {
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess('unused'))
    const response = await handleChat(
      jsonRequest({ message: 'hi' }),
      TEST_ENV,
      { state: {}, aiComplete: aiCompleteMock },
    )
    expect(response.status).toBe(503)
    const body = await response.json() as {
      errors: Array<{ code: string; verb: string; _links: { worker: { href: string } } }>
      status: number
      retryAfter: string
    }
    expect(body.errors[0]!.code).toBe('chat.no_body')
    expect(body.errors[0]!.verb).toBe('chat')
    expect(body.errors[0]!._links.worker.href).toBe(CHAT_WORKER_URL)
    expect(body.status).toBe(503)
    expect(body.retryAfter).toContain('/arest/chat')
    // aiComplete must NOT be called when there's no binding to dispatch
    expect(aiCompleteMock).not.toHaveBeenCalled()
  })

  it('includes Retry-After header on the 503 no-body path (mirror of kernel #620)', async () => {
    const response = await handleChat(
      jsonRequest({ message: 'hi' }),
      TEST_ENV,
      { state: {}, aiComplete: vi.fn(aiCompleteMockSuccess('unused')) },
    )
    expect(response.status).toBe(503)
    expect(response.headers.get('retry-after')).toBeTruthy()
  })

  it('partial Agent Definition (model present, prompt missing) yields 503 with introspectable metadata', async () => {
    const partialState: AgentVerbState = {
      Verb: [{ id: 'verb-chat', name: 'chat' }],
      Verb_invokes_Agent_Definition: [
        { Verb: 'verb-chat', 'Agent Definition': 'agent-chatter' },
      ],
      Agent_Definition_uses_Model: [
        { 'Agent Definition': 'agent-chatter', Model: 'gpt-4o-mini' },
      ],
      // Prompt missing — resolver returns null, but introspection
      // surfaces the Model so dashboards can see the partial config.
      Agent_Definition_has_Prompt: [],
    }
    const response = await handleChat(
      jsonRequest({ message: 'hi' }),
      TEST_ENV,
      { state: partialState, aiComplete: vi.fn(aiCompleteMockSuccess('unused')) },
    )
    expect(response.status).toBe(503)
    const body = await response.json() as {
      errors: Array<{ code: string; agentDefinition?: { model?: string; prompt?: string } }>
    }
    expect(body.errors[0]!.code).toBe('chat.no_body')
    expect(body.errors[0]!.agentDefinition?.model).toBe('gpt-4o-mini')
  })

  // ── 503 path: aiComplete failure bubble-up ──────────────────────────

  it('returns 503 envelope with bubbled-up aiComplete error code on auth failure', async () => {
    const response = await handleChat(
      jsonRequest({ message: 'hi' }),
      TEST_ENV,
      { state: CHAT_AGENT_STATE, aiComplete: aiCompleteMockError('auth', 'invalid token') },
    )
    expect(response.status).toBe(503)
    const body = await response.json() as {
      errors: Array<{ code: string; message: string; verb: string; agentDefinition?: { model?: string; agentDefinitionId?: string } }>
    }
    expect(body.errors[0]!.code).toBe('chat.ai_complete.auth')
    expect(body.errors[0]!.message).toContain('invalid token')
    expect(body.errors[0]!.verb).toBe('chat')
    // Resolved binding info is surfaced for diagnostic purposes
    expect(body.errors[0]!.agentDefinition?.model).toBe('gpt-4o-mini')
    expect(body.errors[0]!.agentDefinition?.agentDefinitionId).toBe('agent-chatter')
  })

  it('returns 503 envelope with bubbled-up aiComplete error code on upstream failure', async () => {
    const response = await handleChat(
      jsonRequest({ message: 'hi' }),
      TEST_ENV,
      { state: CHAT_AGENT_STATE, aiComplete: aiCompleteMockError('upstream', '502 bad gateway') },
    )
    expect(response.status).toBe(503)
    const body = await response.json() as { errors: Array<{ code: string }> }
    expect(body.errors[0]!.code).toBe('chat.ai_complete.upstream')
  })

  it('returns 503 envelope with bubbled-up aiComplete error code on network failure', async () => {
    const response = await handleChat(
      jsonRequest({ message: 'hi' }),
      TEST_ENV,
      { state: CHAT_AGENT_STATE, aiComplete: aiCompleteMockError('network', 'fetch failed') },
    )
    expect(response.status).toBe(503)
    const body = await response.json() as { errors: Array<{ code: string; message: string }> }
    expect(body.errors[0]!.code).toBe('chat.ai_complete.network')
    expect(body.errors[0]!.message).toContain('fetch failed')
  })

  it('returns 503 envelope with bubbled-up aiComplete error code on config failure', async () => {
    const response = await handleChat(
      jsonRequest({ message: 'hi' }),
      TEST_ENV,
      { state: CHAT_AGENT_STATE, aiComplete: aiCompleteMockError('config', 'AI_GATEWAY_URL missing') },
    )
    expect(response.status).toBe(503)
    const body = await response.json() as { errors: Array<{ code: string }> }
    expect(body.errors[0]!.code).toBe('chat.ai_complete.config')
  })

  // ── Malformed input ─────────────────────────────────────────────────

  it('returns 503 with chat.parse code on malformed JSON body (mirror of kernel #620)', async () => {
    const badRequest = new Request('https://arest.do/arest/chat', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{not json',
    })
    const response = await handleChat(badRequest, TEST_ENV, {
      state: CHAT_AGENT_STATE,
      aiComplete: vi.fn(aiCompleteMockSuccess('unused')),
    })
    expect(response.status).toBe(503)
    const body = await response.json() as { errors: Array<{ code: string; verb: string }> }
    expect(body.errors[0]!.code).toBe('chat.parse')
    expect(body.errors[0]!.verb).toBe('chat')
  })

  it('does not call aiComplete on malformed JSON', async () => {
    const aiCompleteMock = vi.fn(aiCompleteMockSuccess('unused'))
    const badRequest = new Request('https://arest.do/arest/chat', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{not json',
    })
    await handleChat(badRequest, TEST_ENV, {
      state: CHAT_AGENT_STATE,
      aiComplete: aiCompleteMock,
    })
    expect(aiCompleteMock).not.toHaveBeenCalled()
  })
})
