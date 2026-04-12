# 09 · MCP Verbs

The MCP (Model Context Protocol) server is how agents interact with graphdl-orm. The v1.0 verb set is frozen in four tiers: **primitive** (the algebra requires them), **entity sugar** (ergonomic shortcuts), **introspection** (read-only metadata), **evolution** (governed self-modification), and **LLM bridge** (natural-language to formal-fact translation via client sampling).

## Configuration

### Claude Code / Claude Desktop

```json
{
  "mcpServers": {
    "graphdl": {
      "command": "npx",
      "args": ["-y", "graphdl-orm", "mcp"],
      "env": {
        "GRAPHDL_MODE": "local",
        "GRAPHDL_READINGS_DIR": "/absolute/path/to/readings"
      }
    }
  }
}
```

### Remote mode (when the server is deployed)

```json
{
  "mcpServers": {
    "graphdl": {
      "command": "npx",
      "args": ["-y", "graphdl-orm", "mcp"],
      "env": {
        "GRAPHDL_MODE": "remote",
        "GRAPHDL_URL": "https://your-domain.com",
        "GRAPHDL_API_KEY": "secret"
      }
    }
  }
}
```

## Primitive verbs

The algebra requires these four. Every entity-level action is expressible as a sequence of them.

### `assert`

Push a single fact into `P`. Triggers resolve, derive, validate, and SM fold as usual.

```typescript
assert({ fact_type: "Order_was_placed_by_Customer", bindings: { Order: "ord-1", Customer: "acme" } })
```

### `retract`

Remove a specific fact from `P`. Distinct from deletion (which is transition to a terminal status — see `delete`).

```typescript
retract({ fact_type: "Order_was_placed_by_Customer", bindings: { Order: "ord-1", Customer: "acme" } })
```

### `project`

Codd's θ₁ projection. Restrict `P` to a fact type and optional filter.

```typescript
project({ fact_type: "Order", filter: { status: "Placed" } })
```

### `compile`

Ingest new readings. Immediate self-modification — the new definitions are merged into `DEFS` and every subsequent call evaluates them. See [self-modification](10-self-modification.md).

```typescript
compile({ readings: "Order(.Order Id) is an entity type.\n..." })
```

## Entity sugar

Ergonomic shortcuts over the primitives. Most agents will use these.

### `get`

Get an entity by ID or list all entities of a noun type. Returns the entity with HATEOAS links and navigation.

```typescript
get({ id: "ord-1", noun: "Order" })
// or
get({ noun: "Order" })   // lists all
```

If the noun is federated, `get` reaches the external system transparently.

### `query`

Query facts across the population, with filters.

```typescript
query({ fact_type: "Order_was_placed_by_Customer", filter: { Customer: "acme" } })
```

### `apply`

Apply the full SYSTEM function with an input. For advanced users.

```typescript
apply({ key: "create:Order", input: "<<Order Id, ord-1>, <Customer, acme>>" })
```

### `create`

Create an entity with field facts.

```typescript
create({ noun: "Order", id: "ord-1", fields: { "Order Id": "ord-1", Customer: "acme" } })
```

Runs the full pipeline: resolve → derive → validate → emit. Returns the entity with HATEOAS links.

### `read`

Same as `get` but name-aligned with CRUD expectations. Returns the full RMAP row.

### `update`

Assert new field facts. Old facts are superseded by new assertions via derivation rules — there is no implicit delete.

### `transition`

Advance a state machine. Takes the entity ID and the event name; the SM fold checks whether the transition is legal from the current status.

```typescript
transition({ noun: "Order", id: "ord-1", event: "place" })
```

### `delete`

Transition the entity to a terminal status. No hard delete by default (see Corollary: Deletion). For fact-level removal, use `retract`.

## Introspection

Read-only calls that describe the running system.

### `explain`

Show the derivation chain for a fact. Returns which rules fired, which antecedents they consumed, and whether each antecedent was asserted or derived.

```typescript
explain({ fact_type: "User_accesses_Domain", bindings: { User: "alice", Domain: "core" } })
```

### `actions`

List the transitions available from the current status of an entity. Equivalent to `_links` in the HATEOAS response.

```typescript
actions({ noun: "Order", id: "ord-1" })
```

### `schema`

Return the schema of a noun or fact type: roles, reference scheme, related fact types, constraints.

```typescript
schema({ noun: "Order" })
schema({ fact_type: "Order_was_placed_by_Customer" })
```

### `verify`

Run constraint evaluation against proposed facts without asserting. Useful for dry-run validation.

```typescript
verify({ fact_type: "Order_was_placed_by_Customer", bindings: { Order: "ord-1", Customer: "acme" } })
```

## Evolution

Governed self-modification.

### `propose`

Create a Domain Change entity with proposed readings, nouns, constraints, or verbs. Enters the review workflow at status `Proposed`.

```typescript
propose({
  rationale: "Add loyalty tier tracking",
  target_domain: "orders",
  readings: ["Customer has Loyalty Tier.\n  Each Customer has exactly one Loyalty Tier."],
  nouns: ["Loyalty Tier"]
})
```

Returns the change ID and next actions: `transition` to `review`, `approve`, `apply`. See [self-modification](10-self-modification.md).

### `compile` (revisited)

Immediate self-modification path. Bypasses the review workflow. Use in trusted contexts (migrations, bootstrap) where proposal review is not needed.

## LLM bridge

Three verbs that use MCP client sampling (via `server.server.createMessage`) to translate between natural language and formal facts. The engine composes the prompt with schema context; the LLM does the translation; the engine executes the formal operation.

### `ask`

Natural-language question to executed projection.

```typescript
ask({ question: "Which orders did acme place in the last week?", noun: "Order" })
```

1. Engine provides the schema to the client LLM.
2. LLM returns a projection spec.
3. Engine executes the projection.
4. Response includes both the query and the results.

### `synthesize`

Facts to prose. Runs the full pipeline (including derive-to-LFP) so derived facts are included, then asks the LLM to verbalize.

```typescript
synthesize({ noun: "Order", id: "ord-1" })
```

The engine guarantees content correctness; the LLM shapes the prose.

### `validate`

Text to constraint check. Useful for document review and content moderation.

```typescript
validate({ text: "Customer Bob placed 3 orders in 5 minutes.", constraint: "rate-limit-orders" })
```

1. Engine fetches the constraint and its fact types.
2. LLM extracts fact instances from the text.
3. Engine runs `verify` on each extracted fact.
4. Response lists any violations.

All three verbs degrade gracefully when the client does not support sampling — they return a prompt the caller can execute itself.

## What is not in the verb set

Runtime function registration (adding new Platform names to the engine's dispatch table) is **not** an MCP verb. Runtime functions are registered server-side at build time. If you need a new named operation, register it as a Platform function in the engine and redeploy; do not expose arbitrary code execution to agents.

## What's next

The last doc. [Self-modification](10-self-modification.md) explains how the system can evolve itself — compile for immediate changes, propose for reviewed ones — without losing the theorems.
