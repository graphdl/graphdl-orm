# GraphDL Tutor

A teaching app that exercises every GraphDL framework concept. Each domain demonstrates specific capabilities. Read the readings, run the MCP server, poke at the system from Claude Code, and build intuition for how fact-oriented modeling produces correct-by-construction applications.

## Quick start

This repo uses [graphdl-orm](https://github.com/graphdl/graphdl-orm) as the runtime. Clone both as siblings:

```bash
# From your projects root
git clone https://github.com/graphdl/graphdl-orm
git clone https://github.com/graphdl/graphdl-tutor

cd graphdl-tutor
yarn install
yarn compile   # round-trips every domain through the Rust compiler
```

To use from Claude Code or Claude Desktop, add this to your MCP config:

```json
{
  "mcpServers": {
    "graphdl-tutor": {
      "command": "yarn",
      "args": ["--cwd", "/absolute/path/to/graphdl-tutor", "mcp"],
      "env": {
        "GRAPHDL_MODE": "local",
        "GRAPHDL_READINGS_DIR": "/absolute/path/to/graphdl-tutor/domains"
      }
    }
  }
}
```

Then ask the agent things like:

> "Create an order for customer alice@example.com, then place it and ship it"
> "Why was this order marked delivered?"
> "Show me every category that is a descendant of Electronics"

Every response is a ρ-application over the facts. No handler code was written; the readings produced the behavior.

## Domains

| Domain | What it teaches |
|--------|-----------------|
| [**_config**](domains/_config.md) | Generator opt-in (SQL, Solidity, iLayer), federation declarations |
| [**catalog**](domains/catalog.md) | Entity and value types, reference schemes, subtypes, objectification, enums, ring constraints (category hierarchy), M:N tags |
| [**orders**](domains/orders.md) | State machines (Order + Payment), transitions, fact-driven events, derivation rules (order total via sum), subset autofill, deontic constraints |
| [**tasks**](domains/tasks.md) | Ternary assignment, priority enums, blocking (acyclic ring), parent-child, comments, milestones, derived completion % |
| [**content**](domains/content.md) | Symmetric ring (related articles), temporal types, word count derivation, publishing lifecycle |
| [**scheduling**](domains/scheduling.md) | Objectification with spanning UC (Booking), recurrence, availability, conflict detection, temporal derivation |
| [**notifications**](domains/notifications.md) | Inclusive-or constraint (at least one channel), delivery state machine with retry, preferences, deontic permissions |

## Suggested learning path

If you are new to fact-oriented modeling, read the domains in this order:

1. **catalog** — the simplest. See how entity types, value types, and constraints fit together. Notice the ring constraint on category hierarchy.
2. **orders** — adds state machines and derivation rules. Watch the `:=` aggregate sum compute the order total from line items automatically.
3. **tasks** — brings in a ternary fact type (Assignment) and an acyclic ring (task blocking). Note the difference between irreflexive, asymmetric, and acyclic.
4. **content** — a symmetric ring constraint (related articles work in both directions).
5. **scheduling** — objectification with a spanning uniqueness constraint. The Booking entity IS the fact about who booked what when.
6. **notifications** — disjunctive-mandatory constraint (every user must have at least one delivery channel), plus deontic permissions.

## Features exercised, with pointers

- **Alethic constraints**: `Each Order has exactly one Amount.` (orders)
- **Deontic constraints**: `It is obligatory that each Product has exactly one Price.` (catalog), `It is forbidden that a Review has Rating less than 1.` (catalog)
- **Derivation rules, aggregate**: `Order has Amount := sum of LineItem Amount where LineItem belongs to that Order.` (orders)
- **Derivation rules, join**: `User accesses Domain := User owns Organization and App belongs to that Organization and Domain belongs to that App.` (metamodel, used via federation)
- **Subset with autofill**: `If some Customer places some Order then that Order has Shipping Address that is that Customer's Shipping Address.` (orders)
- **Compound reference scheme**: `ProductVariant(.id) is an entity type where Product has Size and Color.` (catalog)
- **State machines**: Order has seven statuses and seven transitions (orders)
- **Facts as events**: a fact entering P fires an event the SM consumes (orders, notifications)
- **Federation**: `User is backed by External System 'auth.vin'` — fetched live from the identity provider (_config)
- **Multiple generators**: SQL, Solidity, and iLayer emitted from the same readings (_config)

## Federation credentials

The tutor declares `User` and `Stripe Customer` as federated. Live fetches need secrets:

```bash
export AREST_SECRET_AUTH_VIN='your-auth-vin-api-key'
export AREST_SECRET_STRIPE='sk_test_...'
```

Without them the MCP server still loads the schema and accepts writes to local nouns; federated reads return the empty set (OWA safe default).

## Generators

The tutor opts into SQL (SQLite), Solidity, and iLayer. After compile, each entity has:

- `sql:sqlite:{table}` — CREATE TABLE with UNIQUE, NOT NULL, CHECK constraints and triggers for derivations
- `solidity:{Noun}` — a smart contract with struct, events, `create`, and one function per state machine transition
- `ilayer:{Noun}` — a typed UI layer definition

To regenerate the Solidity specifically:

```bash
yarn compile
cat .out/Generated.sol
```

To get SQL DDL:

```bash
# Via the MCP server, ask the agent: "Give me the SQL schema for the orders domain"
# The agent calls systemCall("sql:sqlite:order", "") etc. for each table.
```

## Writing your own domain

Add a new markdown file under `domains/`. The parser loads every `.md` file in the directory, so your new nouns, fact types, constraints, state machines, and derivation rules become part of the same unified schema.

If you introduce cycles in your derivation rules or contradictions in your constraints, the compiler will reject them with Theorem 3: Completeness. If you declare an entity backed by an external system, it becomes federated automatically.

## License

MIT
