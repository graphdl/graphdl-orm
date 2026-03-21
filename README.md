# graphdl-orm

> "Entia non sunt multiplicanda praeter necessitatem."
> (Entities should not be multiplied beyond necessity.)
> -William of Ockham

A self-describing meta-framework for [Object-Role Modeling](https://en.wikipedia.org/wiki/Object-role_modeling) (ORM2/FORML2). Natural language readings are the source code — they generate relational schemas, APIs, state machines, constraint algebras, UI layouts, and agent prompts.

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

A Durable Object is a row. Individual resources are individual Durable Objects.

```
readings (FORML2 natural language)
    │
    ▼
FORML2 parser (/parse) ──► claim extraction
    │
    ▼
Claim ingestion (/seed) ──► per-domain DomainDB DOs (metamodel)
    │                        + EntityDB DOs (instances)
    │                        + RegistryDB DOs (indexes)
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
    ├── evaluate       ──► constraint verification (Predicate → [Violation])
    ├── forward-chain  ──► FOL inference (Derivation → [DerivedFact])
    ├── query          ──► collection predicate evaluation (Population → [Match])
    └── synthesize     ──► noun knowledge synthesis (SynthesisResult)
    │
    ▼
Consumers: REST API, ui.do, agents (joey), compliance products
```

### DO-Per-Entity Model

Three Durable Object types, each with its own SQLite:

| DO Type | Identity | Purpose |
|---------|----------|---------|
| **EntityDB** | One per entity instance | Single entity: JSON data + version + CDC events |
| **DomainDB** | One per domain slug | Metamodel: nouns, readings, constraints, state machines |
| **RegistryDB** | One per scope (app/org/global) | Indexes: domain registry, noun-to-domain, entity IDs per type |

The Worker is stateless — it routes requests, walks the Registry chain for schema resolution, and runs the FOL engine (WASM) for queries and enrichment.

### Write Path

1. Worker resolves noun schema via Registry chain (app → org → global)
2. Validates input against schema
3. Forward chains subset constraints with autofill (eager enrichment)
4. Creates/updates EntityDB DO
5. Registry indexes the entity

### Read Path (Aggregate Query)

1. Registry provides entity IDs for the noun type
2. Worker fans out RPC to EntityDB DOs in parallel (map)
3. FOL engine evaluates predicates over the collected Population (reduce)
4. FP paradigm ensures parallelism is safe — pure functions over immutable data

## Metamodel

Noun is a subtype of Function. Every entity type, fact type, and constraint is a function. The FOL engine evaluates them. Readings are the interface; FOL is the plumbing.

### Knowledge Layer

| Table | Purpose |
|-------|---------|
| `nouns` | Entity types and value types, with `world_assumption` (closed/open) |
| `graph_schemas` | Fact type definitions |
| `readings` | FORML2 natural language sentences |
| `roles` | Positions in a fact type |
| `constraints` | UC/MC/SS/XC/EQ/OR/XO/IR/AS/AT/SY/IT/TR/AC/FC/VC with modality |
| `constraint_spans` | Maps constraints to roles |

### Behavioral Layer

| Table | Purpose |
|-------|---------|
| `state_machine_definitions` | Lifecycle definitions, references a noun |
| `statuses` | Named states |
| `transitions` | From → to status, triggered by event |
| `guards` | Constraints on transitions |
| `verbs` | Actions/operations |
| `functions` | HTTP callbacks for verbs |

### Scoping

```
Domain is visible to Domain := that Domain is the same Domain.
Domain is visible to Domain := Domain has Visibility 'public'.
Domain is visible to Domain := Domain belongs to App and that Domain belongs to the same App.
Domain is visible to Domain := Domain belongs to Organization and that Domain belongs to the same Organization.
```

Resolution walks the scope chain: local → app → org → global. First match wins.

## FOL Engine (`crates/fol-engine/`)

A first-order logic reasoning engine compiled to WebAssembly, implementing Backus's FP algebra.

Constraints and derivation rules compile to pure functions at load time. Evaluation is function application — no dispatch, no branching on kind, no mutable state.

### Capabilities

**Constraint verification** — All ORM2 constraint kinds: UC, MC, IR, AS, AT, SY, IT, TR, AC, XO, XC, OR, SS, EQ, FC, VC, plus deontic modality (forbidden, obligatory, permitted).

**Forward inference** — Derivation rules (subtype inheritance, modus ponens, transitivity, closed-world negation) applied iteratively until fixed point.

**Collection queries** — `query_population`: evaluate predicates over a Population, return matching entities. The reduce phase of the MapReduce query model.

**Synthesis** — Collect all knowledge about a noun: participating fact types, constraints, state machines, related nouns.

**Dual world assumptions** — Closed World (absence = false) for structural facts. Open World (absence = unknown) for rights and liberties.

## API

```
# Entity CRUD (via EntityDB DOs)
POST   /api/entity                — create entity instance
GET    /api/entities/:noun/:id    — get by ID
PATCH  /api/entities/:noun/:id    — update
DELETE /api/entities/:noun/:id    — delete

# Collection CRUD (metamodel, via DomainDB)
GET    /api/:collection           — list/find (where, limit, page, sort, depth)
GET    /api/:collection/:id       — get by ID
POST   /api/:collection           — create
PATCH  /api/:collection/:id       — update
DELETE /api/:collection/:id       — delete

# Tooling
POST   /api/parse                 — parse FORML2 text into structured claims
POST   /api/generate              — generate artifacts
POST   /api/evaluate              — evaluate text against constraints (WASM)
POST   /api/synthesize            — synthesize knowledge about a noun

# Seeding
POST   /seed                      — bulk seed (parallel per-domain DOs)
GET    /seed                      — stats (fans out to all domain DOs)
DELETE /seed                      — wipe all data
GET    /health                    — health check
```

## Seeding

The seed endpoint accepts FORML2 claims for multiple domains in one call. Each domain gets its own DomainDB DO, seeded in parallel:

```bash
# Parse domain files and POST to seed
curl -X POST https://graphdl-orm.dotdo.workers.dev/seed \
  -H 'Content-Type: application/json' \
  -d '{ "type": "claims", "domains": [{ "slug": "my-domain", "claims": {...} }] }'
```

Phase 1 (metamodel) runs in parallel across DOs. Phase 2 (instance facts) runs after all metamodels are ready. Typical: 25 domains in ~11 seconds.

## Development

```bash
yarn install
yarn dev             # local dev server (wrangler dev)
yarn test            # run tests (vitest) — 583 tests
yarn typecheck       # type check (tsc --noEmit)

# FOL engine
cd crates/fol-engine
cargo test           # 28 tests
```

## Deployment

```bash
yarn deploy          # deploys to Cloudflare Workers
```

### Service Binding

Other Cloudflare Workers connect via service binding:

```typescript
const res = await env.GRAPHDL.fetch(new Request('https://graphdl-orm/api/entities/Customer/abc123'))
```

## License

MIT
