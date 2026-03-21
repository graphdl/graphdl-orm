# graphdl-orm

> "Entia non sunt multiplicanda praeter necessitatem."
> (Entities should not be multiplied beyond necessity.)
> -William of Ockham

A self-describing meta-framework for [Object-Role Modeling](https://en.wikipedia.org/wiki/Object-role_modeling) (ORM2/FORML2). Natural language readings are the source code — they generate relational schemas, APIs, state machines, constraint algebras, and UI layouts.

**Stack:** Cloudflare Workers + Durable Objects (SQLite) + itty-router + Rust/WASM FOL engine

## Intellectual Foundation

This system implements ideas from five foundational works:

- **Codd (1970)** — Data independence: applications derive from the model, not from storage
- **Halpin (ORM2)** — Elementary facts in natural language as the conceptual layer
- **Backus (1977)** — An algebra of programs: constraints compile to pure functions, evaluation is function application over whole structures
- **Bush (1945)** — Associative trails: facts link to facts through readings, not hierarchical indexes
- **Leibniz (1666)** — Characteristica Universalis: a formal language for all knowledge, disputes resolved by calculation

The readings are the source. Everything else is compilation.

## Architecture

```
readings (FORML2 natural language)
    │
    ▼
FORML2 parser (/parse) ──► claim extraction
    │
    ▼
Claim ingestion (/claims) ──► 3NF SQLite in Durable Object
    │
    ▼
Generators (/generate)
    ├── schema      ──► SQLite DDL
    ├── openapi     ──► JSON Schema / REST API
    ├── xstate      ──► state machine configs
    ├── business-rules ──► constraint IR (JSON)
    ├── ilayer      ──► UI layout definitions
    ├── readings    ──► FORML2 round-trip
    └── readme      ──► self-documenting
    │
    ▼
FOL Engine (Rust/WASM)
    ├── evaluate    ──► constraint verification (Predicate → [Violation])
    ├── forward-chain ──► FOL inference (Derivation → [DerivedFact])
    └── synthesize  ──► noun knowledge synthesis (SynthesisResult)
    │
    ▼
Consumers: REST API, ui.do, agents (joey), compliance products
```

A single Durable Object (`GraphDLDB`) holds all metamodel + instance data in normalized SQLite tables. The Worker routes HTTP to the DO via itty-router.

## Three-Layer Schema

### Metamodel (Knowledge Layer)

Describes *what exists* — entity types, fact types, constraints.

| Table | Purpose |
|-------|---------|
| `organizations` | Multi-tenancy root |
| `domains` | Scoped knowledge domains |
| `nouns` | Entity types and value types, with `world_assumption` (closed/open) |
| `graph_schemas` | Fact type definitions |
| `readings` | FORML2 natural language sentences |
| `roles` | Positions in a fact type, references noun + graph_schema |
| `constraints` | UC/MC/SS/XC/EQ/OR/XO/RC with modality (Alethic/Deontic) |
| `constraint_spans` | Maps constraints to roles |

### Behavioral Layer (State)

Describes *how things change* — state machines, transitions, guards.

| Table | Purpose |
|-------|---------|
| `state_machine_definitions` | Lifecycle definitions, references a noun |
| `statuses` | Named states |
| `event_types` | Triggers for transitions |
| `transitions` | From → to status, triggered by event |
| `guards` | Constraints on transitions |
| `verbs` | Actions/operations |
| `functions` | HTTP callbacks for verbs |

### Instance Layer (Runtime)

Describes *what happened* — concrete facts, running state machines, events.

| Table | Purpose |
|-------|---------|
| `graphs` | Fact type instances |
| `resources` | Entity instances |
| `resource_roles` | Bindings of resources to graph roles |
| `state_machines` | Runtime state machines with current status |
| `events` | State machine events that occurred |
| `cdc_events` | Change Data Capture for audit/sync |

## FOL Engine (`crates/fol-engine/`)

A first-order logic reasoning engine compiled to WebAssembly, implementing Backus's FP algebra.

Constraints and derivation rules compile to pure functions at load time. Evaluation is function application — no dispatch, no branching on kind, no mutable state. The algebra's laws (associativity of composition, distributivity of apply-to-all) hold by construction.

### Capabilities

**Constraint verification** — Apply all compiled predicates, collect violations. Supports ORM2 constraint kinds: UC, MC, RC, XO, XC, OR, SS, EQ, plus deontic modality (forbidden, obligatory, permitted).

**Forward inference** — Apply derivation rules (subtype inheritance, modus ponens, transitivity, closed-world negation) iteratively until fixed point. Given base facts, derive all conclusions.

**Synthesis** — Collect all knowledge about a noun: participating fact types, applicable constraints, state machines, related nouns, derived facts. Returns compact summaries for agent context injection.

**Dual world assumptions** — Closed World (absence = false) for government powers and structural facts. Open World (absence = unknown) for individual rights and liberties. Encodes the 9th and 10th Amendments as formal parameters of the reasoning engine.

### Usage

```bash
# Verify text against constraints
fol --ir constraints.json --text "response to verify"

# Synthesize knowledge about a noun
fol --ir constraints.json --synthesize "AI System" --depth 2

# Run forward inference
fol --ir constraints.json --forward-chain --population facts.json
```

See [`crates/fol-engine/README.md`](crates/fol-engine/README.md) for the full theoretical foundation and architecture.

## API

Payload CMS-compatible REST API on all collections:

```
GET    /api/:collection          — list/find (where, limit, page, sort, depth)
GET    /api/:collection/:id      — get by ID
POST   /api/:collection          — create
PATCH  /api/:collection/:id      — update
DELETE /api/:collection/:id      — delete

POST   /api/parse                — parse FORML2 text into structured claims
POST   /api/generate             — generate artifacts (schema, openapi, xstate, business-rules, ilayer, readings)
POST   /api/evaluate             — evaluate text against constraint IR via WASM
POST   /api/synthesize           — synthesize all knowledge about a noun

POST   /seed                     — bulk seed claims
POST   /claims                   — alias for /seed
GET    /seed                     — stats
DELETE /seed                     — wipe all data
GET    /health                   — health check
```

### Query Language

Supports Payload-style `where` bracket notation with FK traversal:

```
/api/nouns?where[objectType][equals]=entity&limit=20&sort=-createdAt
/api/readings?where[domain.domainSlug][equals]=joey&depth=2
/api/nouns?where[name][like]=%State%
```

Operators: `equals`, `not_equals`, `in`, `like`, `exists`, logical `and`/`or`, dot-notation for FK subqueries.

## Claim Ingestion

The `/claims` endpoint accepts structured claims extracted from natural language. The ingestion engine:

1. Creates nouns (find-or-create by name + domain)
2. Applies subtypes (sets `super_type_id`)
3. Creates graph schemas + readings + roles (auto-tokenized)
4. Applies constraints (UC, MC, SS, etc.)
5. Seeds state machine definitions + statuses + transitions
6. Auto-detects world assumption (nouns named Right, Freedom, Liberty, Protection, Privilege → open world)

Derivation rules (`:=` predicates) are stored as readings and resolved dynamically at query time.

## Hooks

Collection write hooks trigger side effects deterministically — creating a reading auto-tokenizes it into roles, creating a constraint auto-spans it to roles, etc.

## Self-Description

On first boot, the Durable Object seeds the `graphdl-core` domain with entity type nouns for every table. The framework can query the metamodel about the metamodel.

## Development

```bash
yarn install
yarn dev             # local dev server (wrangler dev)
yarn test            # run tests (vitest)
yarn typecheck       # type check (tsc --noEmit)

# FOL engine
cd crates/fol-engine
cargo test           # 27 tests
cargo build --release --target wasm32-unknown-unknown  # WASM build
```

## Deployment

```bash
yarn deploy          # deploys to Cloudflare Workers
```

Requires `wrangler` CLI and Cloudflare account access.

### Service Binding

Other Cloudflare Workers connect via service binding (no auth):

```typescript
const res = await env.GRAPHDL.fetch(new Request('https://graphdl-orm/api/nouns?limit=10'))
```

## License

MIT
