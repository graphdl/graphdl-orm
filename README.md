# graphdl-orm

> "Entia non sunt multiplicanda praeter necessitatem." (Entities should not be multiplied beyond necessity.)
>
> -William of Ockham

An implementation of [Object-Role Modeling](https://en.wikipedia.org/wiki/Object-role_modeling) (ORM2/FORML2) as a runtime system. Write your domain in natural language readings. The framework compiles them into relational schemas, REST APIs, state machines, constraint evaluation, UI layouts, and formal proofs.

Based on the work of [Terry Halpin](https://en.wikipedia.org/wiki/Terry_Halpin) (ORM2), [E.F. Codd](https://en.wikipedia.org/wiki/Edgar_F._Codd) (relational model), and [John Backus](https://en.wikipedia.org/wiki/John_Backus) (FP algebra). The readings are the source. Everything else is compilation.

**Stack:** Cloudflare Workers + Durable Objects (SQLite) + itty-router + Rust/WASM FOL engine

## How It Works

```
readings (FORML2 natural language)
    │
    ├─► /parse    ──► structured claims
    ├─► /claims   ──► CSDP validation ──► EntityDB DOs (metamodel eats itself)
    ├─► /generate ──► schemas, APIs, state machines, UIs, docs
    ├─► /query    ──► natural language queries over live data
    ├─► /evaluate ──► constraint verification (FOL engine)
    ├─► /induce   ──► discover constraints from data
    └─► /seed     ──► bulk domain ingestion
```

You write **readings** — natural language sentences that describe your domain. The framework parses them, validates them against Halpin's CSDP, runs inductive constraint discovery, maps them to relational schemas via RMAP, and stores them as individual Durable Object entities. The readings are the source. Everything else is compilation.

## Core Principles

- **Readings are the only authoritative source.** All artifacts — schemas, APIs, state machines, UIs — are projections.
- **The metamodel eats its own tail.** Noun definitions, Reading definitions, Constraint definitions are themselves entities stored in EntityDB DOs, indexed by RegistryDB — the same way Customer and Order instances are stored.
- **Function is the base object.** Noun is a subtype of Function. Verb is a subtype of Function. Everything is a function (Backus).
- **No silent paths.** Every evaluation produces valid claims, Violation entities, or Failure entities. All three are first-class, queryable facts in the same ontology.
- **Transitions define the command surface.** Tools, buttons, and API actions are projections of valid state machine transitions.

## Writing Readings

A readings file is a Markdown document with structured sections. This is the source of truth.

```markdown
# Support Tickets

## Entity Types

Customer(.Email) is an entity type.
Support Request(.Request Id) is an entity type.
Priority is a value type.
  The possible values of Priority are 'Low', 'Medium', 'High', 'Urgent'.

## Fact Types

Customer submits Support Request.
  Each Support Request is submitted by at most one Customer.

Support Request has Priority.
  Each Support Request has exactly one Priority.

Support Request has Description.
  Each Support Request has at most one Description.

## Deontic Constraints

It is forbidden that Support Request has Priority 'Urgent' and Support Request has no assigned Agent.

## Derivation Rules

Support Request is escalated := Support Request has Priority 'Urgent'.

## Transitions

| From | To | Event |
|------|----|-------|
| Open | In Progress | assign |
| In Progress | Resolved | resolve |
| Resolved | Closed | close |
| Resolved | Open | reopen |
```

### ORM2 Constraint Taxonomy

The framework supports the full ORM2 constraint taxonomy with verbalization patterns sourced from Halpin's "ORM 2 Constraint Verbalization Part 1" (TechReport ORM2-02, 2006):

| Concept | Syntax | Example |
|---------|--------|---------|
| **Entity type** | `Name(.Reference) is an entity type.` | `Customer(.Email) is an entity type.` |
| **Value type** | `Name is a value type.` | `Priority is a value type.` |
| **Enum** | `The possible values of X are 'A', 'B'.` | `The possible values of Priority are 'Low', 'High'.` |
| **Subtypes** | `X is a subtype of Y.` | `VIP Customer is a subtype of Customer.` |
| **Partition** | `{X, Y} are mutually exclusive subtypes of Z.` | `{Male, Female} are mutually exclusive subtypes of Person.` |
| **Fact type** | `Noun verb Noun.` | `Customer submits Support Request.` |
| **Uniqueness (UC)** | `Each X has at most one Y.` | `Each Customer has at most one Name.` |
| **Inverse UC** | `For each Y, at most one X [verb] that Y.` | `For each Chair, at most one Academic holds that Chair.` |
| **Spanning UC** | `Each X, Y combination occurs at most once in the population of X verb Y.` | (Halpin TechReport ORM2-02, p.8) |
| **N-1 UC (ternary)** | `For each A and B that A R that B at most one C.` | `For each Student and Course that Student in that Course obtained at most one Rating.` |
| **Mandatory (MC)** | `Each X has some Y.` | `Each Customer has some Name.` |
| **Exactly one** | `Each X has exactly one Y.` (UC + MC) | `Each Customer has exactly one Email.` |
| **Frequency (FC)** | `Each X occurs exactly N times.` | `Each Activity occurs exactly 2 times.` |
| **Value (VC)** | Enum declarations | (see above) |
| **Inverse reading** | `A verb B / B verb A.` | `Academic uses Extension / Extension is used by Academic.` |
| **Objectification** | `This association with A, B provides the preferred identification scheme for X.` | `This association with Academic, Subject provides the preferred identification scheme for Teaching.` |
| **Ring (irreflexive)** | `No X [verb] the same X.` | `No Academic audits the same Academic.` |
| **Ring (asymmetric)** | `If X1 verb X2, then X2 is not verb X1.` | `If Person1 is parent of Person2, then Person2 is not parent of Person1.` |
| **Ring (symmetric)** | `If X1 verb X2, then X2 verb X1.` | `If Person1 is married to Person2, then Person2 is married to Person1.` |
| **Ring (transitive)** | `If X1 verb X2 and X2 verb X3, then X1 verb X3.` | `If A is ancestor of B and B is ancestor of C, then A is ancestor of C.` |
| **Ring (acyclic)** | Implies irreflexive + asymmetric | Common for hierarchies |
| **Subset (SS)** | `If some X verb some Y then that X verb that Y.` | `If some Academic heads some Department then that Academic works for that Department.` |
| **Exclusion (XC)** | `No X both verb1 Y and verb2 Z.` | |
| **Exclusive-or (XO)** | `For each X, exactly one of the following holds: ...` | |
| **Inclusive-or (OR)** | `Each X verb1 some Y or verb2 some Z.` | `Each Lecturer is contracted until some Date or is tenured.` |
| **Deontic** | `It is forbidden/obligatory/permitted that ...` | `It is forbidden that Support Response contains Prohibited Punctuation.` |
| **Derivation** | `X has Y := condition.` | `Person has Full Name := Person has First Name + ' ' + Person has Last Name.` |
| **World assumption** | CWA (default) / OWA | `Noun has World Assumption. The possible values of World Assumption are 'closed', 'open'.` |

## CSDP Validation Pipeline

Claims ingestion follows Halpin's seven-step Conceptual Schema Design Procedure (CSDP) with automated validation. Invalid schemas are rejected with proposed fixes.

**CSDP Step 4 — Arity check:** Ternary fact types with UC spanning fewer than n-1 roles are rejected. Proposed fix: split into binaries.

**CSDP Step 5 — Mandatory roles:** Induction discovers mandatory constraints from the population.

**CSDP Step 6 — Subtypes:** Subtypes declared without totality or exclusion constraints are flagged.

**CSDP Step 7 — Ring constraints:** Self-referential binaries without ring constraints are flagged. Non-elementary facts (and-test) are flagged. Undeclared nouns in constraints are flagged.

```bash
# Claims ingestion with CSDP validation
curl -X POST http://localhost:8787/api/claims \
  -H 'Content-Type: application/json' \
  -d '{ "claims": { ... }, "domain": "support" }'
# → { "valid": true, "batch": { "entities": [...] }, "tables": [...] }
# → or: { "valid": false, "violations": [{ "type": "arity_violation", "message": "...", "fix": "..." }] }
```

## Progressive Induction

The FOL engine discovers constraints from data at three points:

1. **After deterministic parse** — discovers UC/MC/FC/SS patterns from instance facts. Seeds the LLM with discovered patterns.
2. **After LLM extraction** — runs again on the merged population. Higher confidence from more instances.
3. **During CSDP validation** — authoritative pass over the full population including derived facts.

```bash
# Direct induction endpoint
curl -X POST http://localhost:8787/api/induce \
  -H 'Content-Type: application/json' \
  -d '{ "ir": { ... }, "population": { ... } }'
# → { "constraints": [{ "kind": "UC", "confidence": 0.9, "evidence": "..." }], "rules": [...] }
```

## RMAP (Relational Mapping)

The validated conceptual schema is mapped to a relational schema following Halpin's RMAP procedure (Chapter 10, "Information Modeling and Relational Databases"):

- **Step 0.1:** Binarize exclusive unaries → status column with CHECK
- **Step 0.3:** Subtype absorption into root supertype
- **Step 1:** Compound UC → separate table (M:N binaries, ternaries)
- **Step 2:** Functional roles → grouped into entity table
- **Step 3:** 1:1 → absorb, favor fewer nulls
- **Step 4:** Independent entity → single-column table
- **Step 6:** UCs → keys, MCs → NOT NULL, SS → FK, value → CHECK, ring → CHECK/trigger

```bash
# Generate artifacts from RMAP output
curl -X POST http://localhost:8787/api/generate \
  -H 'Content-Type: application/json' \
  -d '{ "domainId": "uuid", "outputFormat": "sqlite" }'
```

| Format | Output |
|--------|--------|
| `openapi` | REST API specification (OpenAPI 3.0 with JSON Schema) |
| `sqlite` | Relational DDL (CREATE TABLE statements with constraints) |
| `xstate` | XState state machine configurations |
| `ilayer` | UI layout definitions (controls, grids, menus) |
| `readings` | FORML2 round-trip (reconstructed from metamodel) |
| `readme` | Self-documenting markdown |
| `mdxui` | MDX UI component definitions |
| `schema` | Domain schema (input for the FOL engine) |

## State Machines as Command Surface

Any entity type can have a lifecycle. Transitions define what can happen — tools, buttons, and API actions are projections of valid transitions from the current state.

**Entity responses include transitions:**
```bash
curl http://localhost:8787/api/entities/SupportRequest/uuid?domain=support
# → {
#   "id": "uuid", "type": "SupportRequest", "data": { ... },
#   "state": "Open",
#   "transitions": [
#     { "event": "assign", "targetStatus": "In Progress", "guards": [] },
#     { "event": "escalate", "targetStatus": "Escalated", "guards": ["requires-manager"] }
#   ]
# }
```

Three projections of the same data:
- **API:** `POST /api/entities/:type/:id/transition { event: "assign" }`
- **UI:** render `transitions` as action buttons or inline menu items
- **Agent:** receive `transitions` as dynamically generated LLM tools

**Cascade pipeline:** When a transition fires and its Verb (a Function) has a callback URI, the framework:
1. Executes the callback
2. Matches the HTTP response status against Event Type Patterns (e.g., `2XX`, `4XX`, `*`)
3. If a match is found, fires the next transition automatically
4. Repeats until no match or a final state

**Fire a transition:**
```bash
curl -X POST http://localhost:8787/api/entities/SupportRequest/uuid/transition \
  -H 'Content-Type: application/json' \
  -d '{ "event": "assign", "domain": "support" }'
# → { "status": "In Progress", "cascade": { "statesVisited": [...] }, "availableEvents": ["resolve", "escalate"] }
```

## Failures as Facts

Violations and failures are first-class domain entities, not out-of-band error responses. Every evaluation path persists its outcomes as queryable EntityDB DOs.

- **Violation** — semantic invalidity. `Violation is of Constraint. Violation is against Function.`
- **Failure** — execution failure. `Failure has Failure Type. Failure is against Function.`

Causal and temporal links:
- `Failure is caused by Violation.`
- `Violation is triggered by Resource.`
- `Failure occurs during Transition.`
- `Failure follows Violation.`
- `Violation occurs before Transition.`

**Deontic enforcement on writes:** Entity creation checks deontic constraints. Forbidden violations reject the write (422). Obligatory warnings allow the write but persist Violation entities.

## Constraint Evaluation (FOL Engine)

The FOL engine (Rust compiled to WASM) evaluates all ORM2 constraint types against a population:

```bash
curl -X POST http://localhost:8787/api/evaluate \
  -H 'Content-Type: application/json' \
  -d '{ "domainId": "uuid", "response": { "text": "..." }, "population": { ... } }'
# → { "violations": [{ "constraintId": "...", "constraintText": "...", "detail": "..." }] }
```

The complete reasoning cycle: **Observe** (facts) → **Induce** (rules from data) → **Deduce** (forward chain to fixpoint) → **Prove** (backward chain with proof trees) → **Evaluate** (constraint check).

**Synthesis** — get everything the framework knows about an entity type:
```bash
curl -X POST http://localhost:8787/api/synthesize \
  -H 'Content-Type: application/json' \
  -d '{ "domainId": "uuid", "nounName": "Support Request" }'
```

## Architecture

Three Durable Object types, each backed by SQLite:

| DO | Granularity | Stores |
|----|-------------|--------|
| **EntityDB** | One per entity instance | Data (JSON blob), version, CDC events. Every metamodel entity (Noun, Reading, Constraint) and every domain instance (Customer, Order) is an EntityDB DO. |
| **DomainDB** | One per domain | Batch WAL (transactional integrity for ingestion) + generators cache. |
| **RegistryDB** | One per scope (org/public) | Domain registry, noun-to-domain index, entity-to-domain index (with `domain_slug` for per-domain queries). |

The metamodel eats its own tail: seeding the `core` domain produces EntityDB DOs for Noun, Reading, Constraint, etc. — the same way seeding a `tickets` domain produces EntityDB DOs for Customer and SupportRequest.

**Write path:**
```
FORML2 text → parseFORML2() → ExtractedClaims
  → CSDP validation (Steps 1-7, with induction at Steps 4-6)
  → if valid: RMAP → BatchBuilder → DomainDB.commitBatch() (atomic WAL)
  → materializeBatch() → fan-out to EntityDB DOs + RegistryDB indexes
  → forward-chain derivation rules → evaluate constraints
```

**Read path:**
```
GET /api/entities/Noun?domain=tickets
  → RegistryDB.getEntityIds('Noun', 'tickets')
  → fan-out to EntityDB DOs (batches of 50, Promise.allSettled)
  → in-memory filter/paginate → JSON response
```

**Scope:** Two-tier resolution (org → public). Apps lasso which org domains they use; they don't own domains.

### Type Hierarchy

```
Function(.id)
  ├── Noun
  │   ├── Graph Schema (objectified fact type)
  │   └── Status
  └── Verb (state transformer, has callback URI)

Constraint(.id)
  ├── Set Comparison Constraint
  └── Frequency Constraint

Reading(.id)
Role(.id)
```

### Intellectual Foundation

- **Codd (1970)** — Data independence: applications derive from the model, not from storage
- **Halpin (ORM2)** — Elementary facts in natural language as the conceptual layer
- **Backus (1977)** — An algebra of programs: constraints compile to pure functions
- **Bush (1945)** — Associative trails: facts link to facts through readings
- **Leibniz (1666)** — Characteristica Universalis: a formal language for all knowledge

## API Reference

```
# Entity CRUD
POST   /api/entity                     — create entity instance (with deontic check + state machine init)
GET    /api/entities/:type             — list by type (requires ?domain=)
GET    /api/entities/:type/:id         — get by ID (includes transitions if state machine exists)
PATCH  /api/entities/:type/:id         — update
DELETE /api/entities/:type/:id         — soft-delete (with cascade)

# State Machine
GET    /api/entities/:type/:id/transitions  — available transitions
POST   /api/entities/:type/:id/transition   — fire transition (with cascade pipeline)

# Natural Language Query
GET    /api/query?q=...&domain=...     — conceptual query
POST   /api/query                      — conceptual query (body)

# Claims & Validation
POST   /api/claims                     — ingest claims with CSDP validation
POST   /api/induce                     — discover constraints from population
GET    /api/stats                      — entity counts by type/domain

# Tooling
POST   /parse                          — parse FORML2 text → structured claims
POST   /parse/orm                      — parse NORMA ORM XML → structured claims
POST   /api/generate                   — generate artifacts (openapi, sqlite, xstate, etc.)
POST   /api/evaluate                   — validate against constraints (WASM FOL engine)
POST   /api/synthesize                 — noun knowledge synthesis
POST   /verify                         — verify prose against domain nouns

# Seeding
POST   /seed                           — bulk seed domains from text or pre-parsed claims
GET    /seed                           — seed stats
DELETE /seed                           — wipe all data

# System
GET    /health                         — health check
GET    /ws                             — WebSocket (CDC events on batch commit)
GET    /debug/table/:table             — entity counts by type from Registry
```

## Development

```bash
yarn install
yarn dev             # local dev server (wrangler dev)
yarn test            # run tests (vitest) — 888 tests across 69 files
yarn typecheck       # type check (tsc --noEmit)

# Seed the core metamodel domain
npx tsx scripts/seed-core.ts

# FOL engine (Rust/WASM)
cd crates/fol-engine
cargo test           # 59 Rust tests (28 lib + 28 bin + 3 integration)
```

## License

MIT
