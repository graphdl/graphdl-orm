/**
 * Tests for the worker boot-time Agent Definition seed (#641 / Worker-AI-4).
 *
 * The seed populates the agents-metamodel cells so the dispatch chain
 * Verb→AgentDef→Model→Prompt resolves at runtime for both `extract`
 * (#639, landed) and `chat` (#640, in flight) verbs. Without it the
 * cell walker `resolveAgentVerb` always returns `null` and the worker
 * 503s on every request. Mirror of the kernel's `system::init` Agent
 * Definition seed pattern documented at
 * `crates/arest-kernel/src/system.rs:262` ("baked one in").
 *
 * The verification contract here is small but precise: each cell
 * required by `resolveAgentVerb` (Verb / Verb_invokes_Agent_Definition /
 * Agent_Definition_uses_Model / Agent_Definition_has_Prompt) has the
 * exact role-name keys the resolver looks up — `id`, `name`, `Verb`,
 * `Agent Definition`, `Model`, `Prompt`. Subtle role-binding drift
 * (e.g. lowercased keys) would make the seed silently invisible to
 * the resolver. These tests pin the contract.
 *
 * The introspection cells (`Agent_Definition_has_Name`,
 * `Agent_Definition_belongs_to_Domain`) aren't required by the
 * resolver but are baked in too — they round out the metamodel
 * footprint per `readings/templates/agents.md` and surface in
 * dashboards / `/explain` endpoints once those query the seed
 * directly.
 */

/// <reference types="node" />
import { describe, it, expect } from 'vitest'
import { handleExtract } from './extract'
import {
  AGENT_DEFINITIONS_STATE,
  EXTRACTOR_AGENT_ID,
  CHATTER_AGENT_ID,
} from './agent-seed'
import { resolveAgentVerb } from './extract'
import type { AiCompleteResult } from './complete'

const TEST_ENV = {
  AI_GATEWAY_URL: 'https://gateway.ai.cloudflare.com/v1/test-acct/test-gw/openai',
  AI_GATEWAY_TOKEN: 'test-token-123',
}

describe('AGENT_DEFINITIONS_STATE — worker boot-time seed (#641)', () => {
  // ── resolveAgentVerb integration: both verbs must resolve ──────────

  it('the extract verb resolves to the Extractor Agent Definition', () => {
    const binding = resolveAgentVerb(AGENT_DEFINITIONS_STATE, 'extract')
    expect(binding).not.toBeNull()
    expect(binding?.agentDefinitionId).toBe(EXTRACTOR_AGENT_ID)
    expect(binding?.modelCode).toBeTruthy()
    expect(binding?.prompt).toBeTruthy()
  })

  it('the chat verb resolves to the Chatter Agent Definition', () => {
    const binding = resolveAgentVerb(AGENT_DEFINITIONS_STATE, 'chat')
    expect(binding).not.toBeNull()
    expect(binding?.agentDefinitionId).toBe(CHATTER_AGENT_ID)
    expect(binding?.modelCode).toBeTruthy()
    expect(binding?.prompt).toBeTruthy()
  })

  it('an unknown verb returns null (no accidental cross-binding)', () => {
    expect(resolveAgentVerb(AGENT_DEFINITIONS_STATE, 'summarise')).toBeNull()
  })

  // ── Cell shape: each required cell has the right role-name keys ────

  it('Verb cells use { id, name } binding shape', () => {
    const verbs = AGENT_DEFINITIONS_STATE.Verb!
    expect(verbs.length).toBeGreaterThanOrEqual(2)
    for (const v of verbs) {
      expect(typeof v.id).toBe('string')
      expect(typeof v.name).toBe('string')
    }
  })

  it('Verb_invokes_Agent_Definition cells use { Verb, "Agent Definition" } binding shape', () => {
    const facts = AGENT_DEFINITIONS_STATE.Verb_invokes_Agent_Definition!
    for (const f of facts) {
      expect(typeof f.Verb).toBe('string')
      expect(typeof f['Agent Definition']).toBe('string')
    }
  })

  it('Agent_Definition_uses_Model cells use { "Agent Definition", Model } shape', () => {
    const facts = AGENT_DEFINITIONS_STATE.Agent_Definition_uses_Model!
    for (const f of facts) {
      expect(typeof f['Agent Definition']).toBe('string')
      expect(typeof f.Model).toBe('string')
    }
  })

  it('Agent_Definition_has_Prompt cells use { "Agent Definition", Prompt } shape', () => {
    const facts = AGENT_DEFINITIONS_STATE.Agent_Definition_has_Prompt!
    for (const f of facts) {
      expect(typeof f['Agent Definition']).toBe('string')
      expect(typeof f.Prompt).toBe('string')
      expect(f.Prompt.length).toBeGreaterThan(0)
    }
  })

  // ── Introspection cells: not required for dispatch, baked anyway ───

  it('Agent_Definition_has_Name cell exists for every Agent Definition', () => {
    const names = AGENT_DEFINITIONS_STATE.Agent_Definition_has_Name!
    expect(names.length).toBeGreaterThanOrEqual(2)
    const namedAgents = new Set(names.map(f => f['Agent Definition']))
    expect(namedAgents.has(EXTRACTOR_AGENT_ID)).toBe(true)
    expect(namedAgents.has(CHATTER_AGENT_ID)).toBe(true)
    expect(names.find(f => f['Agent Definition'] === EXTRACTOR_AGENT_ID)?.Name).toBe('Extractor')
    expect(names.find(f => f['Agent Definition'] === CHATTER_AGENT_ID)?.Name).toBe('Chatter')
  })

  it('Agent_Definition_belongs_to_Domain cell binds both agents to the "ai" Domain', () => {
    const domains = AGENT_DEFINITIONS_STATE.Agent_Definition_belongs_to_Domain!
    expect(domains.length).toBeGreaterThanOrEqual(2)
    for (const f of domains) {
      expect(f.Domain).toBe('ai')
      expect(typeof f['Agent Definition']).toBe('string')
    }
  })

  // ── End-to-end: handleExtract returns 200 against the seed ─────────

  it('handleExtract returns 200 with parsed JSON when given the seed + a successful aiComplete mock', async () => {
    const fixture = { name: 'Acme Corp', amount: 1200.5 }
    const aiCompleteMock = async (): Promise<AiCompleteResult> => ({
      text: JSON.stringify(fixture),
      _meta: { gateway: TEST_ENV.AI_GATEWAY_URL, model: 'gpt-4o-mini' },
    })
    const request = new Request('https://arest.do/arest/extract', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ text: 'invoice from Acme Corp for $1200.50' }),
    })
    const response = await handleExtract(request, TEST_ENV, {
      state: AGENT_DEFINITIONS_STATE,
      aiComplete: aiCompleteMock,
    })

    // Without the seed this is 503 forever (the original bug #641
    // resolves). With the seed we round-trip a 200.
    expect(response.status).toBe(200)
    const body = await response.json() as { data: unknown }
    expect(body.data).toEqual(fixture)
  })

  it('handleExtract returns 503 with extract.ai_complete.config when env vars are missing — proves the chain resolved past the resolver', async () => {
    // Same seed, but the actual aiComplete (no mock) is called against
    // an empty-string env. This is the verification scenario from the
    // task: "If [AI_GATEWAY_URL+TOKEN] are absent, expect 503 with the
    // introspection envelope showing the Agent Definition details
    // (proves the chain resolved through Agent Definition correctly;
    // only the upstream LLM call failed)."
    const emptyEnv = { AI_GATEWAY_URL: '', AI_GATEWAY_TOKEN: '' }
    const request = new Request('https://arest.do/arest/extract', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ text: 'whatever' }),
    })
    const response = await handleExtract(request, emptyEnv, {
      state: AGENT_DEFINITIONS_STATE,
      // Use the real aiComplete (no override) — config check fires first.
    })
    expect(response.status).toBe(503)
    const body = await response.json() as {
      errors: Array<{ code: string; agentDefinition?: { model?: string; agentDefinitionId?: string } }>
    }
    expect(body.errors[0]!.code).toBe('extract.ai_complete.config')
    // The agentDefinition block in this branch carries the resolved
    // ids — proves the resolver walked all four cells successfully.
    expect(body.errors[0]!.agentDefinition?.agentDefinitionId).toBe(EXTRACTOR_AGENT_ID)
  })
})
