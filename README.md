# graphdl-orm

> "Entia non sunt multiplicanda praeter necessitatem." (Entities should not be multiplied beyond necessity.)
>
> -William of Ockham

A meta-framework for [Object-Role Modeling](https://en.wikipedia.org/wiki/Object-role_modeling) (ORM2/FORML2). Write your domain in natural language. Get a relational schema, REST API, state machines, constraint validation, UI layouts, and agent prompts — compiled, not configured.

**Stack:** Cloudflare Workers + Durable Objects (SQLite) + itty-router + Rust/WASM FOL engine

## How It Works

```
readings (FORML2 natural language)
    │
    ├─► /parse    ──► structured claims
    ├─► /seed     ──► live domain (metamodel + instances)
    ├─► /generate ──► schemas, APIs, state machines, UIs, docs
    ├─► /query    ──► natural language queries over live data
    └─► /evaluate ──► constraint verification (FOL engine)
```

You write **readings** — natural language sentences that describe your domain. The framework parses them into a metamodel, then compiles that metamodel into whatever artifact you need.

The readings are the source. Everything else is compilation.

## Writing Readings

A readings file is a Markdown document with structured sections. This is the source of truth — everything else is derived.

```markdown
# Support Tickets

## Entity Types

Customer(.Email) is an entity type.
Support Request(.Request Id) is an entity type.
Priority is a value type.
  The possible values of Priority are 'Low', 'Medium', 'High', 'Urgent'.

## Fact Types

Customer submits Support Request.
  Each Customer submits zero or more Support Requests.
  Each Support Request is submitted by exactly one Customer.

Support Request has Priority.
  Each Support Request has exactly one Priority.

Support Request has Description.
  Each Support Request has at most one Description.

## Deontic Constraints

It is forbidden that Support Request has Priority 'Urgent' and Support Request has no assigned Agent.

## Derivation Rules

Support Request is escalated := Support Request has Priority 'Urgent'.

## States

Support Request has states: Open, In Progress, Resolved, Closed.

## Transitions

Open -> In Progress (assign).
In Progress -> Resolved (resolve).
Resolved -> Closed (close).
Resolved -> Open (reopen).
```

### Key Concepts

| Concept | Syntax | Example |
|---------|--------|---------|
| **Entity type** | `Name(.Reference) is an entity type.` | `Customer(.Email) is an entity type.` |
| **Value type** | `Name is a value type.` | `Priority is a value type.` |
| **Enum** | `The possible values of X are 'A', 'B'.` | `The possible values of Priority are 'Low', 'High'.` |
| **Subtypes** | `X is a subtype of Y.` | `VIP Customer is a subtype of Customer.` |
| **Fact type** | `Noun verb Noun.` | `Customer submits Support Request.` |
| **Uniqueness** | `Each X has at most one Y.` | `Each Support Request has at most one Priority.` |
| **Mandatory** | `Each X has some Y.` | `Each Support Request has some Priority.` |
| **Deontic** | `It is forbidden/obligatory that ...` | `It is forbidden that Support Request has Priority 'Urgent' and Support Request has no assigned Agent.` |
| **Derivation** | `X has Y := condition.` | `Support Request is escalated := Support Request has Priority 'Urgent'.` |

The framework supports the full ORM2 constraint taxonomy — uniqueness, mandatory, frequency, subset, equality, exclusion, inclusive/exclusive or, ring constraints (irreflexive, asymmetric, antisymmetric, symmetric, intransitive, transitive, acyclic), value comparison, and deontic modality (forbidden, obligatory, permitted).

## Seeding a Domain

Feed your readings to the framework. It parses and ingests them into a live, queryable domain.

**From raw FORML2 text** (parsed server-side):
```bash
curl -X POST https://graphdl-orm.dotdo.workers.dev/seed \
  -H 'Content-Type: application/json' \
  -d '{
    "type": "claims",
    "domain": "support",
    "text": "# Support Tickets\n\n## Entity Types\n\nCustomer(.Email) is an entity type.\n..."
  }'
```

**From pre-parsed claims** (multiple domains in one call):
```bash
curl -X POST https://graphdl-orm.dotdo.workers.dev/seed \
  -H 'Content-Type: application/json' \
  -d '{
    "type": "claims",
    "domains": [
      { "slug": "support", "name": "Support Tickets", "claims": { ... } },
      { "slug": "billing", "name": "Billing", "claims": { ... } }
    ]
  }'
```

**Check what's seeded:**
```bash
curl https://graphdl-orm.dotdo.workers.dev/seed
```

Each domain gets its own isolated storage. Seeding runs in parallel across domains — 25 domains in ~11 seconds.

## Creating Entities

Once a domain is seeded, create entity instances against it:

```bash
curl -X POST https://graphdl-orm.dotdo.workers.dev/api/entity \
  -H 'Content-Type: application/json' \
  -d '{
    "noun": "Customer",
    "domain": "support",
    "fields": { "email": "alice@example.com", "name": "Alice" }
  }'
# → { "id": "uuid", "noun": "Customer", "domain": "support", "version": 1 }
```

On write, the framework automatically:

1. Resolves the noun schema from the domain metamodel
2. Initializes the state machine (if the noun has one)
3. Fires derivation rules and writes derived facts
4. Indexes the entity for querying

**Nested entities** — array-of-objects fields become separate entities with FK references back to the parent:

```bash
curl -X POST https://graphdl-orm.dotdo.workers.dev/api/entity \
  -H 'Content-Type: application/json' \
  -d '{
    "noun": "SupportRequest",
    "domain": "support",
    "fields": {
      "priority": "High",
      "description": "Login broken",
      "comments": [
        { "text": "Tried resetting password", "author": "alice@example.com" }
      ]
    }
  }'
```

## Querying

### By ID

```bash
curl https://graphdl-orm.dotdo.workers.dev/api/entities/Customer/uuid-here
```

### List with Filters

```bash
curl 'https://graphdl-orm.dotdo.workers.dev/api/entities/Customer?domain=support&where[priority][equals]=High&limit=10&page=1'
```

### Natural Language Queries

Query using the same vocabulary as your readings:

```bash
# "Customer that submits Support Request that has Priority 'High'"
curl 'https://graphdl-orm.dotdo.workers.dev/api/query?q=Customer+that+submits+Support+Request+that+has+Priority+High&domain=support'
```

The query engine splits on `that`, resolves each noun against declared readings, walks FK relationships, and applies value filters.

## Generating Artifacts

The metamodel compiles to multiple output formats:

```bash
curl -X POST https://graphdl-orm.dotdo.workers.dev/api/generate \
  -H 'Content-Type: application/json' \
  -d '{ "domainId": "uuid", "outputFormat": "openapi" }'
```

| Format | Output |
|--------|--------|
| `openapi` | REST API specification (JSON Schema + endpoints) |
| `schema` | JSON Schema for all entity types |
| `sqlite` | Relational DDL (CREATE TABLE statements) |
| `xstate` | XState state machine configurations |
| `business-rules` | Constraint IR (input for the FOL engine) |
| `ilayer` | UI layout definitions (controls, grids, menus) |
| `readings` | FORML2 round-trip (reconstructed from metamodel) |
| `readme` | Self-documenting markdown |
| `mdxui` | MDX UI component definitions |

## State Machines

Any entity type can have a lifecycle. Define states and transitions in your readings — the framework enforces them at runtime.

**Get available transitions:**
```bash
curl 'https://graphdl-orm.dotdo.workers.dev/api/entities/SupportRequest/uuid/transitions?domain=support'
# → { "currentStatus": "Open", "transitions": [{ "event": "assign", "targetStatus": "In Progress" }] }
```

**Fire a transition:**
```bash
curl -X POST https://graphdl-orm.dotdo.workers.dev/api/entities/SupportRequest/uuid/transition \
  -H 'Content-Type: application/json' \
  -d '{ "event": "assign", "domain": "support" }'
# → { "previousStatus": "Open", "status": "In Progress", "event": "assign" }
```

Invalid transitions return an error with the list of valid events from the current state.

## Constraint Evaluation

Validate data against business rules using the FOL engine (Rust/WASM):

```bash
curl -X POST https://graphdl-orm.dotdo.workers.dev/api/evaluate \
  -H 'Content-Type: application/json' \
  -d '{
    "domainId": "uuid",
    "response": { "text": "...", "fields": { "priority": "High" } }
  }'
# → [{ "constraintId": "...", "text": "Each Support Request has exactly one Priority", "violation": "..." }]
```

### Synthesis

Get everything the framework knows about an entity type — fact types, constraints, state machines, derivation rules, related nouns:

```bash
curl -X POST https://graphdl-orm.dotdo.workers.dev/api/synthesize \
  -H 'Content-Type: application/json' \
  -d '{ "domainId": "uuid", "nounName": "Support Request" }'
```

## Parsing

Parse FORML2 text into structured claims without seeding:

```bash
curl -X POST https://graphdl-orm.dotdo.workers.dev/api/parse \
  -H 'Content-Type: application/json' \
  -d '{ "text": "Customer(.Email) is an entity type.\nCustomer has Name.", "domain": "test" }'
# → { "nouns": [...], "readings": [...], "constraints": [...], "coverage": { ... } }
```

Returns parsed claims, coverage metrics, and any unparsed lines.

## Metamodel API

The metamodel itself is queryable as collections — useful for introspection, tooling, and building UIs on top of the domain model:

```
GET    /api/:collection           — list/find (where, limit, page, sort, depth)
GET    /api/:collection/:id       — get by ID
POST   /api/:collection           — create
PATCH  /api/:collection/:id       — update
DELETE /api/:collection/:id       — delete
```

Collections: `nouns`, `readings`, `graph-schemas`, `roles`, `constraints`, `constraint-spans`, `state-machine-definitions`, `statuses`, `transitions`, `guards`, `verbs`, `functions`, `domains`, `generators`

Filter by domain: `?where[domain][equals]=uuid` or `?where[domain.domainSlug][equals]=support`

Depth population follows FK relationships: `?depth=2`

## WebSocket (Live Events)

```javascript
const ws = new WebSocket('wss://graphdl-orm.dotdo.workers.dev/ws?domain=support')
ws.onmessage = (e) => console.log(JSON.parse(e.data))
```

## Service Binding

Other Cloudflare Workers connect via service binding:

```typescript
const res = await env.GRAPHDL.fetch(
  new Request('https://graphdl-orm/api/entities/Customer/abc123')
)
```

## Development

```bash
yarn install
yarn dev             # local dev server (wrangler dev)
yarn test            # run tests (vitest)
yarn typecheck       # type check (tsc --noEmit)

# FOL engine (Rust/WASM)
cd crates/fol-engine
cargo test
```

## Deployment

```bash
yarn deploy          # deploys to Cloudflare Workers
```

## API Reference

```
# Entity CRUD
POST   /api/entity                     — create entity instance
GET    /api/entities/:noun             — list by type (requires ?domain=)
GET    /api/entities/:noun/:id         — get by ID
PATCH  /api/entities/:noun/:id         — update
DELETE /api/entities/:noun/:id         — delete

# State Machine
GET    /api/entities/:noun/:id/transitions  — available transitions
POST   /api/entities/:noun/:id/transition   — fire transition event

# Natural Language Query
GET    /api/query?q=...&domain=...     — conceptual query
POST   /api/query                      — conceptual query (body)

# Metamodel Collections
GET    /api/:collection                — list/find
GET    /api/:collection/:id            — get by ID
POST   /api/:collection                — create
PATCH  /api/:collection/:id            — update
DELETE /api/:collection/:id            — delete

# Tooling
POST   /api/parse                      — parse FORML2 text → claims
POST   /api/generate                   — generate artifacts from metamodel
POST   /api/evaluate                   — validate against constraints (WASM)
POST   /api/synthesize                 — noun knowledge synthesis
POST   /api/facts                      — create instance-level facts
POST   /api/claims                     — ingest claims directly
GET    /api/stats                      — ingestion statistics

# Seeding
POST   /seed                           — bulk seed domains
GET    /seed                           — seed stats
DELETE /seed                           — wipe all data

# System
GET    /health                         — health check
GET    /ws                             — WebSocket (live events)
```

## Architecture

The framework runs on Cloudflare Workers with three Durable Object types, each backed by SQLite:

| DO | Granularity | Stores |
|----|-------------|--------|
| **DomainDB** | One per domain | Metamodel — nouns, readings, constraints, state machines |
| **EntityDB** | One per entity instance | Data, version history, CDC events |
| **RegistryDB** | One per scope | Indexes — domain registry, noun-to-domain, entity IDs |

The Worker is stateless. It routes requests, resolves schemas via the Registry scope chain (app → org → global), and runs the FOL engine (Rust compiled to WASM) for constraint evaluation and forward inference.

On write: schema resolution → eager enrichment (subset constraint autofill) → derivation rules (forward chain to fixpoint) → state machine initialization → index.

On query: Registry provides entity IDs → fan out to EntityDB DOs in parallel (map) → FOL engine evaluates predicates (reduce).

### Intellectual Foundation

- **Codd (1970)** — Data independence: applications derive from the model, not from storage
- **Halpin (ORM2)** — Elementary facts in natural language as the conceptual layer
- **Backus (1977)** — An algebra of programs: constraints compile to pure functions
- **Bush (1945)** — Associative trails: facts link to facts through readings
- **Leibniz (1666)** — Characteristica Universalis: a formal language for all knowledge

## License

MIT
