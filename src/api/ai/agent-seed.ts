/**
 * Worker boot-time Agent Definition seed (#641 / Worker-AI-4).
 *
 * Populates the agents-metamodel cells the dispatch chain
 * Verb→AgentDef→Model→Prompt walks at request time. Without this
 * seed, the migrated /arest/extract path (#639) and /arest/chat path
 * (#640) hit `resolveAgentVerb` returning `null` and 503 forever.
 *
 * Mirror of the kernel's `system::init` Agent Definition seed pattern
 * documented at `crates/arest-kernel/src/system.rs:262` ("baked one
 * in"). The kernel registers `Func::Platform("extract")` at boot so
 * `apply(Func::Def("extract"), …)` resolves through the standard
 * ρ-dispatch path; the worker has no equivalent Func registry —
 * instead the TS port of `agent::resolve_agent_verb` walks four
 * cells off `AgentVerbState` to reach the same binding. This module
 * is the worker's equivalent to the kernel's boot-time cell push.
 *
 * Two Agent Definitions are baked in:
 *
 *   • Extractor — invoked by the `extract` verb. Produces structured
 *     JSON from free text. Default model `gpt-4o-mini` (the AI
 *     Gateway is OpenAI-compatible per `complete.ts`; the gateway
 *     translates for Workers AI / Anthropic upstreams).
 *   • Chatter   — invoked by the `chat` verb (#640's migrated
 *     handler). General-purpose conversational model.
 *
 * Both belong to Domain `ai`. Cell shape is canonical-underscored
 * exactly as the parser cascade emits — same form `extract.test.ts`
 * fixtures use, same form `resolveAgentVerb` reads. Role-binding
 * keys preserve the `Agent Definition` literal (with internal
 * space) per the Halpin reading-name convention.
 *
 * IDs are stable string constants so other modules (e.g. `/explain`,
 * dashboard tools) can reference them without re-deriving.
 *
 * ── Naming ──────────────────────────────────────────────────────────
 *
 * The kernel-side seed at `system.rs:218` uses ids like `sm-sr-1` /
 * `t-categorize` (kebab-case, fixture-style). We follow the same
 * convention: `agent-extractor` / `agent-chatter`. The `ai-` prefix
 * on Verb ids matches the Domain (avoids collision when other
 * domains add a verb of the same name later).
 */

import type { AgentVerbState } from './extract'

// ── Stable IDs (exported for cross-module reference) ────────────────

/**
 * Verb ids. The verb name (`extract`, `chat`) is what
 * `resolveAgentVerb` matches on; the id is what
 * `Verb_invokes_Agent_Definition` rows reference. Using a `verb-`
 * prefix keeps these grep-able and avoids accidental clashes with
 * Agent Definition ids in the same state.
 */
export const EXTRACT_VERB_ID = 'verb-extract'
export const CHAT_VERB_ID = 'verb-chat'

/**
 * Agent Definition ids. Referenced by every cell the resolver and
 * introspection paths walk (`Verb_invokes_Agent_Definition`,
 * `Agent_Definition_uses_Model`, `Agent_Definition_has_Prompt`,
 * `Agent_Definition_has_Name`, `Agent_Definition_belongs_to_Domain`).
 */
export const EXTRACTOR_AGENT_ID = 'agent-extractor'
export const CHATTER_AGENT_ID = 'agent-chatter'

// ── Model defaults ──────────────────────────────────────────────────
//
// Both agents default to `gpt-4o-mini` — the AI Gateway is an
// OpenAI-compatible endpoint per `complete.ts`'s wire shape, and the
// `mini` tier covers extraction + chat with sub-cent per-request
// economics. Operators wanting a different model for production can
// either override the seed or land a kernel-side dynamic-readings
// pass once #562 (DynRdg-T3 — DO-cell-backed reading store) lands.

const EXTRACTOR_MODEL = 'gpt-4o-mini'
const CHATTER_MODEL = 'gpt-4o-mini'

// ── Prompt templates ────────────────────────────────────────────────
//
// `renderPrompt` in `extract.ts` appends the JSON-stringified request
// body to whatever string lives in `Agent_Definition_has_Prompt`. For
// extract we want strict JSON output; for chat we want natural-prose.
// Keep these concise — the LLM has a token budget and the system
// prompt is the dominant cost on small inputs.

const EXTRACTOR_PROMPT =
  "You are an information-extraction assistant. Read the input and produce a single JSON object " +
  "containing the structured fields the input describes. Reply with ONLY the JSON object — no " +
  "prose, no markdown code fences, no preamble. If a field is unclear or not present in the input, " +
  "omit it from the JSON rather than guessing."

const CHATTER_PROMPT =
  "You are a helpful conversational assistant. Reply concisely and accurately to the user's input. " +
  "Stay on topic, ask a clarifying question if the input is ambiguous, and avoid speculation."

// ── The seed state ──────────────────────────────────────────────────

/**
 * The frozen worker boot state with both Agent Definitions populated.
 *
 * Importers wanting to extend the seed (e.g. injecting a third agent
 * at request time) can spread it: `{ ...AGENT_DEFINITIONS_STATE, … }`.
 * The arrays are read-only by the type system; structural cloning
 * is the caller's responsibility if mutation is needed.
 */
export const AGENT_DEFINITIONS_STATE: AgentVerbState = {
  // Two verbs registered. The `name` is what `resolveAgentVerb`
  // matches against; the `id` is what every other cell references.
  Verb: [
    { id: EXTRACT_VERB_ID, name: 'extract' },
    { id: CHAT_VERB_ID, name: 'chat' },
  ],

  // Wire each verb to its Agent Definition. One row per (verb, agent)
  // pair — the metamodel says "Each Verb invokes at most one Agent
  // Definition" (templates/agents.md), so two rows total.
  Verb_invokes_Agent_Definition: [
    { Verb: EXTRACT_VERB_ID, 'Agent Definition': EXTRACTOR_AGENT_ID },
    { Verb: CHAT_VERB_ID, 'Agent Definition': CHATTER_AGENT_ID },
  ],

  // Each Agent Definition uses exactly one Model. The `Model` value
  // is the model code (slug the AI Gateway accepts), not a Model
  // entity id — `resolveAgentVerb` returns it as `modelCode` and
  // threads it straight into `aiComplete({ model })`.
  Agent_Definition_uses_Model: [
    { 'Agent Definition': EXTRACTOR_AGENT_ID, Model: EXTRACTOR_MODEL },
    { 'Agent Definition': CHATTER_AGENT_ID, Model: CHATTER_MODEL },
  ],

  // Each Agent Definition has exactly one Prompt. The Prompt string
  // is the system-instruction body; `renderPrompt` in extract.ts
  // concatenates `\n\nInput:\n<json-body>` at dispatch time.
  Agent_Definition_has_Prompt: [
    { 'Agent Definition': EXTRACTOR_AGENT_ID, Prompt: EXTRACTOR_PROMPT },
    { 'Agent Definition': CHATTER_AGENT_ID, Prompt: CHATTER_PROMPT },
  ],

  // ── Introspection cells (not consulted by resolveAgentVerb) ──────
  //
  // The metamodel (templates/agents.md) declares "Agent Definition
  // has Name" and "Agent Definition belongs to Domain" with mandatory
  // (1..1) frequency. Bake them so dashboards / `/explain` endpoints
  // see a fully-populated Agent Definition rather than just the
  // dispatch-critical four cells.

  Agent_Definition_has_Name: [
    { 'Agent Definition': EXTRACTOR_AGENT_ID, Name: 'Extractor' },
    { 'Agent Definition': CHATTER_AGENT_ID, Name: 'Chatter' },
  ],

  Agent_Definition_belongs_to_Domain: [
    { 'Agent Definition': EXTRACTOR_AGENT_ID, Domain: 'ai' },
    { 'Agent Definition': CHATTER_AGENT_ID, Domain: 'ai' },
  ],
}
