# GraphDL WASM Constraint Engine + Chat Endpoint

**Date:** 2026-03-12
**Status:** Design approved, pending implementation plan

## Problem

graphdl-orm has a complete metamodel engine that ingests FORML2 readings, creates nouns/roles/constraints/state machines, and generates OpenAPI schemas, SQLite DDL, XState configs, iLayer UI definitions, and FORML2 round-trip text. The recently-added `/parse` and `/verify` endpoints provide deterministic FORML2 parsing and noun-matching verification as pure functions. What it still lacks:

1. **Runtime constraint evaluation** — The ~200 deontic constraints in domains like support.auto.dev (forbidden text, obligatory fields, permitted actions) are stored but never enforced at runtime. The `/verify` endpoint identifies which constraints are relevant to prose, but does not evaluate whether they are satisfied.
2. **A chat endpoint** — ui.do calls `POST /graphdl/chat` for streaming AI responses, but the endpoint doesn't exist. Generated agent prompts and tools sit in the `generators` collection unused.
3. **Constraint-aware AI** — No mechanism to evaluate Claude's draft responses against domain constraints before delivering them to users.

## Solution

Three additions to graphdl-orm, all on a branch:

1. A `constraint-ir` generator that queries the database and emits a JSON intermediate representation of all constraints for a domain.
2. A Rust crate that compiles constraint IR into a `.wasm` module for near-native evaluation on Cloudflare Workers.
3. A `POST /api/chat` endpoint that orchestrates Claude calls with WASM constraint evaluation in a redraft loop, streaming clean responses via SSE.

All existing functionality is preserved unchanged: 29 collection slugs, 4 hooks, 5 output formats (openapi/sqlite/xstate/ilayer/readings), the new `/parse` and `/verify` endpoints, full CRUD, cascade deletes, CDC logging, depth population, claims ingestion, bootstrap, and the full ORM2 constraint vocabulary (UC, MC, RC, SS, XC, EQ, OR, XO).

## Architecture

```
FORML2 Readings
  --> graphdl-orm seed/claims ingestion
    --> hooks create nouns, readings, roles, constraints, constraint-spans, state machines
      --> POST /api/generate { outputFormat: 'constraint-ir' }
        --> constraint-ir generator queries DB, emits JSON IR
          --> Rust crate consumes IR, compiles to .wasm
            --> .wasm deployed with Cloudflare Worker

POST /api/chat { domainId, messages[], requestId? }
  --> Load agent prompt + tools from generators collection
  --> Query population snapshot from DO (SQLite)
  --> Call Claude with prompt, tools, conversation history
  --> WASM module evaluates draft response against constraint IR
  --> If violations: feed back to Claude for redraft (max N iterations)
  --> Stream clean response via SSE
  --> Persist Messages, execute state transitions
```

### Relationship to `/parse` and `/verify`

The chat endpoint complements the recently-implemented pure-function endpoints:

- **`/parse`** — Deterministic FORML2 text → `ExtractedClaims`. Used upstream for ingestion, not at chat time.
- **`/verify`** — Deterministic noun-matching: identifies which constraints are *relevant* to prose text. The chat endpoint can call `verifyProse()` (the pure function) to scope which constraints to evaluate via WASM, reducing the evaluation set.
- **`/api/chat`** — Full constraint *evaluation* (not just relevance detection) via WASM, plus AI orchestration.

The WASM evaluator handles what `/verify` cannot: determining whether a constraint is *satisfied*, not just *relevant*.

## 1. Constraint IR Generator

### Integration Point

New `outputFormat: 'constraint-ir'` value added to `VALID_FORMATS` in `src/api/generate.ts`, dispatching to a new `generateConstraintIR(db, domainId)` function in `src/generate/constraint-ir.ts`.

Follows the exact pattern of existing generators:
- Fetches domain data via DO methods (`findInCollection`, `getFromCollection`)
- Returns structured output
- Persisted to `generators` collection by `handleGenerate()`

### IR Shape

The generator queries the database for:
- All constraints (kind, modality, text, set_comparison_argument_length) scoped to the domain
- Constraint spans linking constraints to roles (including subset_autofill flag)
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
    spans: Array<{ factTypeId: string, roleIndex: number, subsetAutofill?: boolean }>
    // Set-comparison constraints (XO/XC/OR/SS/EQ)
    setComparisonArgumentLength?: number
    clauses?: string[]
    entity?: string
  }>
  stateMachines: Record<string, {
    nounName: string
    statuses: string[]
    transitions: Array<{
      from: string
      to: string
      event: string
      guard?: {
        graphSchemaId: string
        // Resolved at IR generation time: graph_schema → constraint_spans → constraints
        constraintIds: string[]
      }
    }>
  }>
}
```

The IR is self-contained: the WASM module needs no further database access at evaluation time.

**Note on `deonticOperator`:** The `constraints` table stores `kind`, `modality`, `text`, and `set_comparison_argument_length` — it does not persist the deontic operator separately. The generator re-derives `deonticOperator` from the constraint `text` column at generation time using `parseConstraintText()` from `src/hooks/parse-constraint.ts`. No schema migration needed.

**Note on guard → constraint resolution:** The `guards` table references `graph_schema_id`, not `constraint_id`. The generator resolves from `guards.graph_schema_id` → `constraint_spans` (which link constraints to roles on that graph schema) → `constraints`. This join is performed at IR generation time so the WASM module receives pre-resolved constraint IDs.

**Note on set-comparison constraints:** The recently-added ORM2 set-comparison support (SS, XC, EQ, OR, XO) stores `set_comparison_argument_length` on the constraint and `subset_autofill` on constraint_spans. The IR generator includes these fields so the WASM evaluator can handle all 8 constraint kinds.

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

// A fact instance binds noun references to roles in a fact type
struct FactInstance { fact_type_id: String, bindings: Vec<(String, String)> }
// bindings: Vec of (role_noun_name, reference_value) — e.g., ("Customer", "cust-123")

// Evaluation
struct Violation { constraint_id: String, constraint_text: String, detail: String }

fn evaluate(ir: &ConstraintIR, population: &Population) -> Vec<Violation>
```

### Constraint Evaluation Logic

**Alethic constraints** (structural):
- UC (uniqueness): No two facts in the population share the same value for the spanned roles
- MC (mandatory): Every entity instance participates in at least one fact of the spanned type
- RC (ring): Irreflexive/asymmetric/intransitive checks on self-referencing fact types

**Set-comparison constraints** (ORM2):
- XO (exclusive-or): For each entity, exactly one of the clause fact types holds
- XC (exclusive-choice): For each entity, at most one of the clause fact types holds
- OR (inclusive-or): For each entity, at least one of the clause fact types holds
- SS (subset): If a fact of type A holds, then the corresponding fact of type B must also hold
- EQ (equality/biconditional): Fact type A holds if and only if fact type B holds

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

Compiled with `wasm-pack build --target bundler` for Cloudflare Workers compatibility. The `--target bundler` output produces ES module imports that integrate with wrangler's module system. The `.wasm` file is included via wrangler's `rules` configuration (see Section 5).

### WASM Lifecycle on Cloudflare Workers

Workers V8 isolates have no persistent in-memory state across requests. The WASM module lifecycle:

1. **Module instantiation**: The `.wasm` binary is loaded as a WebAssembly module at Worker startup (via wrangler's module imports). This is fast — the binary is pre-compiled.
2. **IR loading**: `load_ir()` is called once per request (or cached in a module-level variable within the isolate's lifetime). The IR JSON is fetched from the `generators` collection.
3. **Evaluation**: `evaluate_response()` runs synchronously per draft response. Stateless — each call receives the full population.
4. **No IR case**: If no constraint IR has been generated for the domain, the chat endpoint skips WASM evaluation and streams Claude's response directly (with a warning in the SSE stream).

### State Machines as Constraints

State machines map to the constraint system rather than requiring a separate runtime:

- **Guards are constraints**: A transition guard references a graph schema; the generator resolves this to constraints via constraint_spans. The guard passes iff the resolved constraints evaluate with zero violations against the current population.
- **Transitions are permitted events**: A transition from status A to status B on event E is a permitted fact: `It is permitted that Resource transitions from A to B when E occurs`, but only when `current_status == A`.
- **Status checks are population queries**: "Is entity X in status Y?" is a population membership check on the state_machines instance table.

The WASM module evaluates guard constraints using the same `evaluate()` function. No separate state machine interpreter needed.

## 3. Chat Endpoint

### Route

`POST /api/chat` in `src/api/router.ts`, handled by `src/api/chat.ts`. Registered before the `*` 404 fallback, following the existing `/api/` prefix convention for endpoints that interact with the DO. The upstream `apis` worker proxies `/graphdl/chat` to this Worker's `/api/chat`.

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

2. **Query population snapshot** from the DO's SQLite:
   - Current state machine status for the request (if requestId provided)
   - Relevant instance facts (graphs, resources, resource-roles)
   - Populate the `{{currentState}}` template variable in the agent prompt
   - Note: The broader @dotdo/db architecture uses ClickHouse OLAP for analytics queries. The chat endpoint queries the DO's SQLite directly for low-latency object retrieval. If ClickHouse integration is wired into this Worker in the future, population queries can be routed there for cross-domain analytics.

3. **Scope constraints** via `verifyProse()`:
   - Call the pure function from `/verify` to identify which constraints are relevant to the conversation context
   - Pass only relevant constraint IDs to the WASM evaluator (optimization, not correctness-critical)

4. **Call Claude** with:
   - System prompt: rendered agent prompt with population context
   - Tools: state machine transition tools
   - Messages: conversation history
   - AI binding from Cloudflare Workers environment (or external API)

5. **Evaluate draft** via WASM:
   - Pass Claude's response text + population to `evaluate_response()`
   - If violations returned: append violation feedback to messages, call Claude again
   - Max redraft iterations (configurable, default 3)
   - **If max iterations exhausted**: Stream the last draft with a `violation_warning` SSE event listing remaining violations. The response is delivered rather than blocked — the constraint system is advisory for deontic constraints, not a hard gate.

6. **Stream response** via SSE:
   - `Content-Type: text/event-stream`
   - `data: { content: "..." }` chunks
   - Final `data: [DONE]`
   - Violation events are internal to the redraft loop and NOT streamed to the client. The client sees only clean content or, if max iterations exhausted, a final `violation_warning` event.

7. **Persist and transition**:
   - Create Message record in collection
   - If Claude invoked a tool (state transition): execute via DO, create Event record
   - Log completion to `completions` collection

### Env Additions

```typescript
export interface Env {
  GRAPHDL_DB: DurableObjectNamespace
  ENVIRONMENT: string
  AI: Ai                    // Cloudflare Workers AI binding
  ANTHROPIC_API_KEY?: string // Fallback: direct Anthropic API
}
```

The WASM module is imported as an ES module (not an env binding):
```typescript
import wasmModule from '../crates/constraint-eval/pkg/constraint_eval_bg.wasm'
```

**wrangler.jsonc additions:**
```jsonc
{
  // Existing config preserved...
  "ai": { "binding": "AI" },
  "rules": [
    { "type": "CompiledWasm", "globs": ["**/*.wasm"], "fallthrough": true }
  ]
}
```

If `ANTHROPIC_API_KEY` is set as a secret (`wrangler secret put ANTHROPIC_API_KEY`), it is used as a fallback when the Workers AI binding is unavailable.

### SSE Format

Compatible with ui.do's existing `streamChat()` parser in `src/api.ts`:

```
data: {"type":"content","content":"Hello..."}

data: {"type":"tool_use","name":"resolve","input":{}}

data: {"type":"content","content":"Hello (final clean response)..."}

data: [DONE]
```

If max redraft iterations exhausted:
```
data: {"type":"violation_warning","violations":["Response contains '—'"],"message":"Delivered with unresolved constraints"}

data: {"type":"content","content":"Hello (best effort)..."}

data: [DONE]
```

## 4. Existing Functionality Preservation

Every existing component is preserved without modification:

### Collections (29 slugs in COLLECTION_TABLE_MAP)
All 29 collection slugs remain: organizations, org-memberships, apps, domains, nouns, graph-schemas, readings, roles, constraints, constraint-spans, state-machine-definitions, statuses, transitions, guards, event-types, verbs, functions, streams, models, agent-definitions, agents, completions, generators, graphs, resources, resource-roles, state-machines, events, guard-runs.

### Hooks (4 registered)
- `nouns` → subtype parsing, parent noun creation
- `readings` → tokenize, create nouns/graph-schemas/roles, delegate constraints
- `state-machine-definitions` → create statuses/event-types/transitions
- `constraints` → NL parsing, set-comparison block parsing, find host reading, create constraint-spans

### Generators (5 existing output formats + 1 new)
- `openapi` → OpenAPI 3.0.0 schema from domain model
- `sqlite` → DDL from OpenAPI schema
- `xstate` → XState machine configs from state machine definitions
- `ilayer` → iLayer UI definitions from entity nouns
- `readings` → FORML2 round-trip text
- **NEW**: `constraint-ir` → JSON IR for WASM compilation

### Pure-Function Endpoints (recently added, preserved)
- `POST /parse` → Deterministic FORML2 parsing via `parseFORML2()` (no DB writes)
- `POST /verify` → Deterministic noun-matching via `verifyProse()` (read-only)

### DO Methods
All GraphDLDB methods preserved:
- `initTables()` with migrations (including set-comparison columns) and bootstrap
- `findInCollection()` with full where clause support (and/or/equals/not_equals/in/like/exists/dot-notation)
- `getFromCollection()`, `createInCollection()`, `updateInCollection()`, `deleteFromCollection()`
- `withWriteLock()` serialization
- `logCdcEvent()` CDC audit trail
- `cascadeDeleteDomain()` leaf-to-root cascade
- `wipeAllData()` for test/reset

### API Routes
All existing routes preserved:
- `GET /health`
- `POST /api/generate` (5 output formats)
- `GET/POST/PATCH/DELETE /api/:collection(/:id)`
- `GET/POST/DELETE /seed` and `/claims`
- `POST /parse`
- `POST /verify`
- `* → 404`

**NEW**: `POST /api/chat`

### Schema DDL
All tables across metamodel, state, agent, and instance schemas preserved, including recent migrations adding `set_comparison_argument_length` to constraints and `subset_autofill` to constraint_spans.

### Constraint System
Full ORM2 constraint vocabulary preserved:
- Alethic: UC (uniqueness), MC (mandatory), RC (ring)
- Set-comparison: SS (subset), XC (exclusive-choice), EQ (equality), OR (inclusive-or), XO (exclusive-or)
- Deontic modality on any constraint kind
- `parseConstraintText()` and `parseSetComparisonBlock()` in `src/hooks/parse-constraint.ts`

### Claims Ingestion
Full `ingestClaims()` pipeline preserved: noun creation, reading creation with graph schemas and roles, constraint application via constraint-spans (supporting all 8 kinds), state machine creation from transitions.

### Tests
All existing test files preserved (parse.test.ts, verify.test.ts, parse-constraint.test.ts, and all prior tests). New tests added for:
- `constraint-ir` generator
- Chat endpoint
- WASM evaluation (Rust tests + integration tests)

## 5. Build & Deploy Pipeline

### Development Flow

```
1. Edit FORML2 readings (e.g., support.auto.dev/domains/support.md)
2. POST /seed with claims → hooks create metamodel entities
3. POST /api/generate { outputFormat: 'constraint-ir' } → JSON IR
4. cd crates/constraint-eval && wasm-pack build --target bundler → .wasm
5. Deploy Worker with .wasm module via wrangler
```

### CI/CD Additions

The existing `.github/workflows/deploy.yml` (yarn install → yarn test → wrangler deploy) extended with:
- Rust toolchain + wasm-pack installation
- `cargo test` in crates/constraint-eval
- `wasm-pack build --target bundler` step
- WASM module included in Worker deployment (wrangler `rules` config handles `.wasm` files)

### Cloudflare Workers Constraints

- No threads → WASM runs synchronously in V8 isolate (fine for constraint evaluation, which is CPU-bound and fast)
- CPU time limits → Constraint evaluation must complete within Worker CPU budget (50ms free, 200ms paid). IR is pre-compiled; evaluation is O(constraints * population_size), well within limits for typical domains.
- WASM support → First-class in Workers, near-native speed
- Memory → WASM linear memory, independent of V8 heap
- Isolate recycling → No persistent in-memory state; IR re-loaded per isolate (but not per request within an isolate's lifetime)

## 6. Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Constraint evaluation runtime | Rust → WASM | Near-native performance on Workers, no JS overhead, type safety |
| No intermediate TypeScript step | Direct to WASM | "This is a branch, and there is already something that works. I don't want intermediate steps." |
| exec-symbols relationship | Architecture only, not implementation | Nouns-as-functions and parallel evaluation are load-bearing concepts; Church-encoded closures are not production-ready |
| IR as generator output | constraint-ir generator in graphdl | "If this is a target, there should be a generator in order to update the app automatically from readings." |
| State machines | Mapped to constraints | Guards → graph_schema → constraint_spans → constraints. Uses existing metamodel, no separate runtime. |
| Storage for chat | DO's SQLite (direct) | Low-latency object retrieval. @dotdo/db ClickHouse integration is the broader architecture for analytics but is not yet wired into this Worker. |
| Streaming format | SSE | ui.do already parses SSE from streamChat() |
| AI provider | Cloudflare Workers AI binding | Native to deployment platform; Anthropic API key as fallback |
| WASM target | `--target bundler` | ES module output integrates with wrangler's module system |
| Max redraft exhaustion | Deliver with warning | Deontic constraints are advisory; blocking response delivery is worse than imperfect compliance |
| Violation visibility | Internal to redraft loop | Client sees clean responses or final warning, not intermediate drafts |
| Constraint scoping | `verifyProse()` pre-filter | Reduces WASM evaluation set using existing pure function |
| deonticOperator derivation | Re-parse from text at IR generation | Avoids schema migration; `parseConstraintText()` already handles this |
| Guard resolution | Join at IR generation time | guards.graph_schema_id → constraint_spans → constraints. WASM receives pre-resolved IDs. |
