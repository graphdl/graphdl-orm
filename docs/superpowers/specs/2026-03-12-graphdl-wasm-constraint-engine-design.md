# GraphDL WASM Constraint Engine + Chat Endpoint

**Date:** 2026-03-12
**Status:** Design approved, pending implementation plan

## Problem

graphdl-orm has a complete metamodel engine that ingests FORML2 readings, creates nouns/roles/constraints/state machines, and generates OpenAPI schemas, SQLite DDL, XState configs, iLayer UI definitions, and FORML2 round-trip text. What it lacks:

1. **Runtime constraint evaluation** — The ~200 deontic constraints in domains like support.auto.dev (forbidden text, obligatory fields, permitted actions) are stored but never enforced at runtime.
2. **A chat endpoint** — ui.do calls `POST /graphdl/chat` for streaming AI responses, but the endpoint doesn't exist. Generated agent prompts and tools sit in the `generators` collection unused.
3. **Constraint-aware AI** — No mechanism to evaluate Claude's draft responses against domain constraints before delivering them to users.

## Solution

Three additions to graphdl-orm, all on a branch:

1. A `constraint-ir` generator that queries the database and emits a JSON intermediate representation of all constraints for a domain.
2. A Rust crate that compiles constraint IR into a `.wasm` module for near-native evaluation on Cloudflare Workers.
3. A `POST /chat` endpoint that orchestrates Claude calls with WASM constraint evaluation in a redraft loop, streaming clean responses via SSE.

All existing functionality (26 collection slugs, 4 hooks, 5 generators, full CRUD, cascade deletes, CDC logging, depth population, claims ingestion, bootstrap) is preserved unchanged.

## Architecture

```
FORML2 Readings
  --> graphdl-orm seed/claims ingestion
    --> hooks create nouns, readings, roles, constraints, constraint-spans, state machines
      --> POST /api/generate { outputFormat: 'constraint-ir' }
        --> constraint-ir generator queries DB, emits JSON IR
          --> Rust crate consumes IR, compiles to .wasm
            --> .wasm deployed with Cloudflare Worker

POST /chat { domainId, messages[], requestId? }
  --> Load agent prompt + tools from generators collection
  --> Query population snapshot (ClickHouse via @dotdo/db)
  --> Call Claude with prompt, tools, conversation history
  --> WASM module evaluates draft response against constraint IR
  --> If violations: feed back to Claude for redraft (max N iterations)
  --> Stream clean response via SSE
  --> Persist Messages, execute state transitions
```

## 1. Constraint IR Generator

### Integration Point

New `outputFormat: 'constraint-ir'` value in `src/api/generate.ts`, dispatching to a new `generateConstraintIR(db, domainId)` function in `src/generate/constraint-ir.ts`.

Follows the exact pattern of existing generators:
- Fetches domain data via DO methods (`findInCollection`, `getFromCollection`)
- Returns structured output
- Persisted to `generators` collection by `handleGenerate()`

### IR Shape

The generator queries the database for:
- All constraints (kind, modality, text) scoped to the domain
- Constraint spans linking constraints to roles
- Roles linking to readings and nouns
- Noun metadata (objectType, enumValues, valueType)
- State machine definitions, statuses, transitions, event types, guards

Output structure (JSON):

```typescript
interface ConstraintIR {
  domain: string
  nouns: Record<string, {
    objectType: 'entity' | 'value'
    enumValues?: string[]
    valueType?: string
    superType?: string
  }>
  factTypes: Record<string, {
    reading: string
    roles: Array<{ nounName: string, roleIndex: number }>
  }>
  constraints: Array<{
    id: string
    kind: 'UC' | 'MC' | 'SS' | 'XC' | 'EQ' | 'OR' | 'XO' | 'RC'
    modality: 'Alethic' | 'Deontic'
    deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
    text: string
    spans: Array<{ factTypeId: string, roleIndex: number }>
  }>
  stateMachines: Record<string, {
    nounName: string
    statuses: string[]
    transitions: Array<{
      from: string
      to: string
      event: string
      guard?: { constraintId: string }
    }>
  }>
}
```

The IR is self-contained: the WASM module needs no further database access at evaluation time.

## 2. Rust WASM Crate

### Location

`crates/constraint-eval/` in the graphdl-orm repo.

### Architecture (exec-symbols inspired)

From exec-symbols, we take the architecture — not the Church-encoded implementation:

- **Nouns as functions**: A Noun is a function from reference to population membership. This enables parallel evaluation — independent noun checks have no data dependencies and can run concurrently.
- **FactTypes as higher-order functions**: A FactType takes noun bindings and returns whether the fact holds in the current population.
- **Constraints as predicates over populations**: Each constraint is a pure function `(Population) -> Vec<Violation>`. No side effects, no mutation.
- **Population as snapshot**: The current state of all facts, passed immutably to constraint evaluation.

### Rust Design

```rust
// Core types
struct Noun { name: String, object_type: ObjectType, enum_values: Option<Vec<String>> }
struct FactType { id: String, reading: String, roles: Vec<Role> }
struct Constraint { id: String, kind: ConstraintKind, modality: Modality, text: String, spans: Vec<Span> }
struct Population { facts: HashMap<String, Vec<FactInstance>> }

// Evaluation
struct Violation { constraint_id: String, constraint_text: String, detail: String }

fn evaluate(ir: &ConstraintIR, population: &Population) -> Vec<Violation>
```

### Constraint Evaluation Logic

**Alethic constraints** (structural):
- UC (uniqueness): No two facts in the population share the same value for the spanned roles
- MC (mandatory): Every entity instance participates in at least one fact of the spanned type
- RC (ring): Irreflexive/asymmetric/intransitive checks on self-referencing fact types

**Deontic constraints** (policy — the primary target for chat evaluation):
- `forbidden`: The response MUST NOT exhibit the constrained pattern
- `obligatory`: The response MUST exhibit the constrained pattern
- `permitted`: The response MAY exhibit the constrained pattern (no violation possible)

For text-based deontic constraints (the support.auto.dev pattern), evaluation is string/pattern matching against the draft response:
- Forbidden ProhibitedText values → substring check
- Forbidden ImplementationDetail values → substring/semantic check
- Obligatory SenderIdentity → field presence check
- Obligatory PricingModel conformance → structural check

### WASM Interface

```rust
#[wasm_bindgen]
pub fn load_ir(ir_json: &str) -> Result<(), JsValue>

#[wasm_bindgen]
pub fn evaluate_response(response_json: &str, population_json: &str) -> String // JSON Vec<Violation>
```

Compiled with `wasm-pack build --target web` for Cloudflare Workers compatibility.

### State Machines as Constraints

State machines map to the constraint system rather than requiring a separate runtime:

- **Guards are constraints**: A transition guard references a constraint; the guard passes iff the constraint evaluates with zero violations against the current population.
- **Transitions are permitted events**: A transition from status A to status B on event E is a permitted fact: `It is permitted that Resource transitions from A to B when E occurs`, but only when `current_status == A`.
- **Status checks are population queries**: "Is entity X in status Y?" is a population membership check on the state_machines instance table.

The WASM module evaluates guard constraints using the same `evaluate()` function. No separate state machine interpreter needed.

## 3. Chat Endpoint

### Route

`POST /chat` in `src/api/router.ts`, handled by `src/api/chat.ts`.

### Request

```typescript
interface ChatRequest {
  domainId: string
  messages: Array<{ role: 'user' | 'assistant', content: string }>
  requestId?: string  // For state machine context
}
```

### Pipeline

1. **Load context** from generators collection:
   - Agent prompt (outputFormat: 'agent-prompt' or from agent-definitions)
   - Tools (outputFormat: 'agent-tools' or from generated xstate)
   - Constraint IR (outputFormat: 'constraint-ir')

2. **Query population snapshot** via @dotdo/db:
   - Current state machine status for the request (if requestId provided)
   - Relevant instance facts from ClickHouse
   - Populate the `{{currentState}}` template variable in the agent prompt

3. **Call Claude** with:
   - System prompt: rendered agent prompt with population context
   - Tools: state machine transition tools
   - Messages: conversation history
   - AI binding from Cloudflare Workers environment (or external API)

4. **Evaluate draft** via WASM:
   - Pass Claude's response text + population to `evaluate_response()`
   - If violations returned: append violation feedback to messages, call Claude again
   - Max redraft iterations (configurable, default 3)

5. **Stream clean response** via SSE:
   - `Content-Type: text/event-stream`
   - `data: { content: "..." }` chunks
   - Final `data: [DONE]`

6. **Persist and transition**:
   - Create Message record in collection
   - If Claude invoked a tool (state transition): execute via DO, create Event record
   - Log completion to `completions` collection

### Env Additions

```typescript
export interface Env {
  GRAPHDL_DB: DurableObjectNamespace
  ENVIRONMENT: string
  AI: Ai                    // Cloudflare Workers AI binding (or Anthropic API key)
  CONSTRAINT_WASM?: WebAssembly.Module  // Pre-compiled WASM module
}
```

### SSE Format

Compatible with ui.do's existing `streamChat()` parser in `src/api.ts`:

```
data: {"type":"content","content":"Hello..."}

data: {"type":"tool_use","name":"resolve","input":{}}

data: {"type":"violation","text":"Response contains prohibited text '—'","retry":1}

data: {"type":"content","content":"Hello (redrafted)..."}

data: [DONE]
```

## 4. Existing Functionality Preservation

Every existing component is preserved without modification:

### Collections (48 slugs in COLLECTION_TABLE_MAP)
All 48 collection slugs remain: organizations, org-memberships, apps, domains, nouns, graph-schemas, readings, roles, constraints, constraint-spans, state-machine-definitions, statuses, event-types, transitions, guards, verbs, functions, streams, models, agent-definitions, agents, completions, graphs, resources, resource-roles, state-machines, events, guard-runs, generators, plus all instance collections.

### Hooks (4 registered)
- `nouns` → subtype parsing, parent noun creation
- `readings` → tokenize, create nouns/graph-schemas/roles, delegate constraints
- `state-machine-definitions` → create statuses/event-types/transitions
- `constraints` → NL parsing, find host reading, create constraint-spans

### Generators (5 existing + 1 new)
- `openapi` → OpenAPI 3.0.0 schema from domain model
- `sqlite` → DDL from OpenAPI schema
- `xstate` → XState machine configs from state machine definitions
- `ilayer` → iLayer UI definitions from entity nouns
- `readings` → FORML2 round-trip text
- **NEW**: `constraint-ir` → JSON IR for WASM compilation

### DO Methods
All GraphDLDB methods preserved:
- `initTables()` with migrations and bootstrap
- `findInCollection()` with full where clause support (and/or/equals/not_equals/in/like/exists/dot-notation)
- `getFromCollection()`, `createInCollection()`, `updateInCollection()`, `deleteFromCollection()`
- `withWriteLock()` serialization
- `logCdcEvent()` CDC audit trail
- `cascadeDeleteDomain()` leaf-to-root cascade
- `wipeAllData()` for test/reset

### API Routes
All existing routes preserved:
- `GET /health`
- `POST /api/generate`
- `GET/POST/PATCH/DELETE /api/:collection(/:id)`
- `GET/POST/DELETE /seed` and `/claims`
- `* → 404`

**NEW**: `POST /chat`

### Schema DDL
All 30+ tables across metamodel, state, agent, and instance schemas preserved. No schema changes to existing tables.

### Claims Ingestion
Full `ingestClaims()` pipeline preserved: noun creation, reading creation with graph schemas and roles, constraint application via constraint-spans, state machine creation from transitions.

### Tests
All 24 existing test files preserved. New tests added for:
- `constraint-ir` generator
- Chat endpoint
- WASM evaluation (Rust tests + integration tests)

## 5. Build & Deploy Pipeline

### Development Flow

```
1. Edit FORML2 readings (e.g., support.auto.dev/domains/support.md)
2. POST /seed with claims → hooks create metamodel entities
3. POST /api/generate { outputFormat: 'constraint-ir' } → JSON IR
4. cd crates/constraint-eval && wasm-pack build --target web → .wasm
5. Deploy Worker with .wasm module
```

### CI/CD Additions

The existing `.github/workflows/deploy.yml` (yarn install → yarn test → wrangler deploy) extended with:
- Rust toolchain + wasm-pack installation
- `cargo test` in crates/constraint-eval
- `wasm-pack build` step
- WASM module included in Worker deployment

### Cloudflare Workers Constraints

- No threads → WASM runs synchronously in V8 isolate (fine for constraint evaluation, which is CPU-bound and fast)
- CPU time limits → Constraint evaluation must complete within Worker CPU budget (50ms free, 200ms paid). IR is pre-compiled; evaluation is O(constraints * population_size), well within limits for typical domains.
- WASM support → First-class in Workers, near-native speed
- Memory → WASM linear memory, independent of V8 heap

## 6. Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Constraint evaluation runtime | Rust → WASM | Near-native performance on Workers, no JS overhead, type safety |
| No intermediate TypeScript step | Direct to WASM | "This is a branch, and there is already something that works. I don't want intermediate steps." |
| exec-symbols relationship | Architecture only, not implementation | Nouns-as-functions and parallel evaluation are load-bearing concepts; Church-encoded closures are not production-ready |
| IR as generator output | constraint-ir generator in graphdl | "If this is a target, there should be a generator in order to update the app automatically from readings." |
| State machines | Mapped to constraints | Guards = constraints, transitions = permitted events. Uses existing metamodel, no separate runtime. |
| Storage | @dotdo/db (ClickHouse + DOs) | Analytics-speed population queries via ClickHouse, DO-performance object retrieval for chat state |
| Streaming format | SSE | ui.do already parses SSE from streamChat() |
| AI provider | Cloudflare Workers AI binding | Native to deployment platform, falls back to Anthropic API |
