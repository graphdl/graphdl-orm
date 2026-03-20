# DO-Per-Entity Architecture

## Problem

The current graphdl-orm architecture puts an entire relational database (31 tables, all domains, all entity instances) into a single Durable Object named "graphdl-primary". This:

- Misuses Durable Objects (a DO is a single entity, not a database server)
- Hits Cloudflare's 30-second request timeout on large operations
- Cannot scale (single writer for all data)
- Has no OLAP layer — all queries, including aggregates, hit one DO's SQLite
- Ignores the existing CDC/ClickHouse infrastructure

## Principles

**A Durable Object IS a row.** Individual resources are individual Durable Objects.

**Readings are the interface.** Queries, constraints, data flow, and enrichment are all expressed as FORML2 readings. The FOL engine is internal plumbing.

**Facts are facts.** When a fact is asserted, all derivable consequences are computed eagerly via forward chaining. A fact is true the moment it's asserted — not deferred until someone asks.

**Subset constraints with autofill are lookups.** "Fills from superset" means: when a new fact arrives, the subset constraint resolves references against the superset population. This is how cross-entity relationships are derived — a message with a phone number automatically resolves to a Customer.

## Architecture

### Four DO Types

| DO Type | Identity Scheme | What It Holds |
|---|---|---|
| **Entity DO** | `env.ENTITY_DB.idFromName(entityId)` | One entity instance: JSON data + version + CDC events. One row per DO. |
| **Domain DO** | `env.DOMAIN_DB.idFromName(domainSlug)` | Metamodel for one domain: nouns, readings, constraints, state machine definitions. |
| **Registry DO** | `env.REGISTRY_DB.idFromName(scope)` where scope is app slug, org slug, or `'global'` | Index of Domain DOs at this scope level + entity ID index per noun type. Resolution routing. |
| **Worker** | Stateless | Request routing, schema resolution, FOL engine (WASM) for forward chaining and queries. |

### Entity DO

A lightweight Durable Object — not @dotdo/db. Custom implementation with:

```sql
-- One row: the entity
CREATE TABLE entity (
  id TEXT PRIMARY KEY,
  type TEXT NOT NULL,
  data TEXT NOT NULL,  -- JSON blob
  version INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  deleted_at TEXT
);

-- CDC audit trail
CREATE TABLE events (
  id TEXT PRIMARY KEY,
  timestamp TEXT NOT NULL DEFAULT (datetime('now')),
  operation TEXT NOT NULL,
  data TEXT,
  prev TEXT
);
```

RPC methods: `get()`, `put(data)`, `patch(fields)`, `delete()`, `events(since?)`.

### Domain DO

The current `GraphDLDB` stripped of instance storage. Keeps the metamodel tables:

- `nouns`, `graph_schemas`, `readings`, `roles`
- `constraints`, `constraint_spans`
- `state_machine_definitions`, `statuses`, `transitions`, `guards`

RPC methods: `findNouns(where)`, `findReadings(where)`, `applySchema()`, `ingestClaims(claims)`.

The `ingestProject` step functions (1-5) target Domain DOs. Each domain's claims go to its Domain DO via `ingestClaims`.

### Registry DO

Three levels matching the scope chain:

- **App Registry** — `env.REGISTRY_DB.idFromName('app:support-auto-dev')`
- **Org Registry** — `env.REGISTRY_DB.idFromName('org:drivly')`
- **Global Registry** — `env.REGISTRY_DB.idFromName('global')`

Internal schema:

```sql
-- Which domains are at this scope level
CREATE TABLE domains (
  domain_slug TEXT PRIMARY KEY,
  domain_do_id TEXT NOT NULL,
  visibility TEXT NOT NULL DEFAULT 'private'
);

-- Noun-to-domain index for fast resolution (avoids N+1 fan-out to Domain DOs)
CREATE TABLE noun_index (
  noun_name TEXT NOT NULL,
  domain_slug TEXT NOT NULL,
  PRIMARY KEY (noun_name, domain_slug)
);

-- Entity ID index per noun type for fan-out queries
CREATE TABLE entity_index (
  noun_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  deleted INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (noun_type, entity_id)
);
```

RPC methods: `resolveNoun(name)`, `registerDomain(slug, doId, visibility)`, `indexEntity(nounType, entityId)`, `deindexEntity(nounType, entityId)`, `getEntityIds(nounType)`.

The `noun_index` table maps noun names to domains so noun resolution is a single Registry lookup, not a fan-out to all Domain DOs.

### Schema Resolution Chain

```
Worker → App Registry → Org Registry → Global Registry
```

1. Worker asks App Registry: `resolveNoun('SupportRequest')`
2. App Registry checks `noun_index` → finds domain slug → returns Domain DO ID
3. If not found, Worker asks Org Registry, then Global Registry
4. Worker gets noun schema from the Domain DO via RPC
5. Worker caches resolved schema (short TTL) to avoid repeated lookups

## Data Flow

### Write Path (Eager Enrichment)

1. Worker receives mutation (create SupportRequest with `phoneNumber: '+1234'`)
2. Worker resolves noun schema via Registry chain
3. Worker validates input against schema
4. **Forward chain**: Worker loads the noun's constraints into FOL engine. For each subset constraint with autofill:
   - Fan out to relevant Entity DOs to build the superset Population
   - FOL engine derives new facts (e.g., phone number → Customer association)
   - Enriched data includes derived associations
5. Worker creates/gets Entity DO, stores enriched data + CDC event
6. Worker tells Registry DO to index this entity ID

Facts are facts — all derivable consequences are computed at write time.

### Read Path (Single Entity)

1. Worker gets Entity DO by ID → returns JSON blob
2. Already enriched at write time — no lazy derivation needed

### Read Path (Aggregate Query)

1. Reading-level query arrives (e.g., `Support Request has Status 'Investigating'`)
2. Worker parses reading into fact type + bound values via `parseFORML2`
3. Worker asks Registry DO for all entity IDs of the noun type
4. Worker fans out RPC to Entity DOs in parallel, collects data
5. Worker builds Population from results
6. FOL engine evaluates the predicate — filters the Population
7. Worker returns matching entities

The FOL engine is the plumbing. Readings are the query language.

### Fan-Out Batching

Fan-out RPC uses configurable batch sizes (default 500) to stay within Worker memory limits. Each batch is collected and reduced before the next batch fires. For very large populations, results stream through the FOL engine incrementally.

### Write Path Detail: Subset Autofill

When a subset constraint has `autofill: true`:

```
Message has Phone Number.
Customer has Phone Number.
  If Message has Phone Number and Customer has Phone Number
  then that Message is from that Customer.
  [fills from superset]
```

On write of a Message with `phoneNumber: '+1234'`:
1. FOL engine identifies the SS constraint with autofill
2. Worker fans out to Customer Entity DOs (via Registry) to find which Customer has that phone number
3. FOL engine derives: `Message is from Customer 'john'`
4. The derived fact is stored on the Message Entity DO alongside the original data

## Seeding Flow

The `ingestProject` pipeline from this session is reused:

- **Steps 1-5** (nouns, subtypes, readings, constraints, transitions) → Domain DOs via Registry. Each domain's claims go to its Domain DO.
- **Step 5.5** (applySchema) → Domain DOs generate their schemas internally.
- **Step 6** (instance facts) → Worker orchestration → Entity DOs. Each fact creates/updates an individual Entity DO, with eager enrichment via forward chaining.

The Scope object and step functions need an adapter layer: instead of calling `db.findInCollection('nouns', ...)` directly, they call Domain DO RPC methods. The step function signatures stay the same; the `db` parameter becomes an adapter that routes to the appropriate DO.

## FOL Engine Extensions

The FOL engine (`crates/fol-engine/`) currently exports:

- `evaluate_response` — constraint violation checking
- `forward_chain_population` — derivation until fixed point
- `synthesize_noun` — knowledge collection about a noun
- `load_ir` — compile constraint IR

New export needed:

- `query_population(ir, population, predicate)` → filtered/aggregated results

This uses the same predicate compilation infrastructure. The existing `instances_of`, `participates_in`, and `compile_set_comparison` (which already does count aggregation) provide the foundation. The new function evaluates a collection predicate over a Population and returns matching entities instead of violations.

The query predicate is compiled from a parsed reading with bound values — same IR format, different evaluation mode.

## Implementation Notes

### WASM Concurrency (I1)

Cloudflare Workers are single-threaded per isolate. The FOL engine's global `OnceLock<CompiledState>` is safe as long as `load_ir` and evaluation calls are not interleaved across `await` points. In practice: load IR, evaluate synchronously, then await the next async operation. Do not `await` between `load_ir` and `evaluate_response`/`query_population`.

### Entity DO Uses SQLite, Not KV (I2)

Each Entity DO holds one entity in a one-row SQLite table. This is intentional — the `events` table genuinely benefits from SQL (time-range queries, ordering), and having both entity and events in the same transactional SQLite ensures atomicity between data writes and CDC event logging.

### Registry DO Scaling (I3)

The `entity_index` table in a single Registry DO could become a write bottleneck at scale. If a noun type exceeds ~100K entities, consider sharding the Registry by noun type (e.g., `env.REGISTRY_DB.idFromName('app:support-auto-dev:SupportRequest')`). This is a future optimization — start with one Registry per scope level.

## CDC Event Flow

Entity DOs log CDC events on every mutation:

```json
{
  "operation": "create",
  "timestamp": "2026-03-20T...",
  "data": { "current entity state" },
  "prev": null
}
```

CDC events can optionally forward to ClickHouse when infrastructure is available. The system works without ClickHouse — the FOL engine provides aggregate query capability using only DOs + fan-out.

## Wrangler Configuration

```jsonc
{
  "name": "graphdl-orm",
  "main": "src/index.ts",
  "compatibility_date": "2026-03-09",
  "compatibility_flags": ["nodejs_compat"],
  "durable_objects": {
    "bindings": [
      { "name": "ENTITY_DB", "class_name": "EntityDB" },
      { "name": "DOMAIN_DB", "class_name": "DomainDB" },
      { "name": "REGISTRY_DB", "class_name": "RegistryDB" }
    ]
  },
  "migrations": [
    { "tag": "v2", "new_sqlite_classes": ["EntityDB", "DomainDB", "RegistryDB"], "deleted_classes": ["GraphDLDB"] }
  ],
  "rules": [
    { "type": "CompiledWasm", "globs": ["**/*.wasm"], "fallthrough": true }
  ]
}
```

## What Gets Reused

| Component | Status |
|---|---|
| `parseFORML2` | Unchanged — parses domain .md files and reading-level queries |
| `generateOpenAPI`, `generateSQLite` | Unchanged — generates schemas from readings |
| `ingestProject` pipeline + Scope | Steps 1-5 target Domain DOs via adapter; step 6 targets Entity DOs via Worker |
| Step functions (`steps.ts`) | Same signatures, `db` parameter becomes DO adapter |
| Metamodel readings | Unchanged — Noun as subtype of Function, Scope, visibility |
| FOL engine constraint compilation | Unchanged — same IR, same predicate model |
| FOL engine `forward_chain_population` | Reused for eager enrichment on writes and query reduction on reads |

## What Gets Replaced

| Current | New |
|---|---|
| Monolithic `GraphDLDB` (31 tables, all data) | Domain DO (metamodel) + Entity DOs (instances) + Registry DOs (indexes) |
| `createEntity` in do.ts (310 lines) | Worker orchestration → Entity DO RPC + FOL enrichment |
| `queryEntities` in do.ts | Reading-level query → fan-out → FOL filter/aggregate |
| Single `graphdl-primary` DO | N Entity DOs + M Domain DOs + Registry DOs per scope level |
| No OLAP | FOL engine as edge-native MapReduce; optional ClickHouse via CDC |
| Manual relationship wiring | Subset constraint autofill — relationships derived eagerly via forward chaining |

## What Gets Built New

| Component | What |
|---|---|
| `EntityDB` DO class | Lightweight DO: entity table, events table, CRUD RPC methods (~100 lines) |
| `DomainDB` DO class | GraphDLDB stripped of instance storage, RPC interface for metamodel queries |
| `RegistryDB` DO class | Scope-level index: domains, noun index, entity index, resolution RPC |
| DO adapter for step functions | Implements `findInCollection`/`createInCollection` interface, routes to appropriate DO |
| `query_population` WASM export | New FOL engine function: collection predicate evaluation over Population |
| Worker orchestration layer | Schema resolution, eager enrichment, fan-out query coordination |

## Testing

1. **Entity DO**: Create/update/delete via RPC, CDC event logging, version incrementing
2. **Domain DO**: Metamodel CRUD, schema generation, noun resolution — retarget existing tests
3. **Registry DO**: Domain registration, noun indexing, entity indexing, resolution chain
4. **Worker integration**: Full write path (resolve → enrich → store), full read path (fan-out → FOL reduce)
5. **FOL query_population**: Predicate evaluation over collections, filtering, aggregation
6. **Eager enrichment**: Subset autofill resolves cross-entity relationships on write
7. **Seeding**: ingestProject with DO adapter targets, verify Domain DOs and Entity DOs created correctly
8. **Fan-out batching**: Verify large noun populations are batched correctly
9. **Backward compatibility**: Parser, generators, step functions, scope — existing 504 tests pass
