# AREST Tutor

This is a teaching app that exercises every AREST framework concept. Each domain demonstrates specific capabilities. Read the readings, run the MCP server, poke at the system from Claude Code, and build intuition for how fact-oriented modeling produces correct-by-construction applications.

## Quick start

The tutor ships as a subfolder of the main AREST repo, so one clone covers everything:

```bash
git clone https://github.com/drivly/arest
cd arest/tutor
yarn install
yarn compile   # round-trips every domain through the Rust compiler
```

To use the tutor from Claude Code or Claude Desktop, add this to your MCP config (the AREST repo already contains a `.mcp.json` with a matching `arest-tutor` entry):

```json
{
  "mcpServers": {
    "arest-tutor": {
      "command": "yarn",
      "args": ["--cwd", "/absolute/path/to/arest/tutor", "mcp"],
      "env": {
        "AREST_MODE": "local",
        "AREST_READINGS_DIR": "/absolute/path/to/arest/tutor/domains"
      }
    }
  }
}
```

Then ask the agent things like:

> "Create an order for customer alice@example.com, then place it and ship it."
> "Why was this order marked delivered?"
> "Show me every category that is a descendant of Electronics."

Every response is a ρ-application over the facts. No handler code was written; the readings produced the behavior.

## Domains

| Domain | What it teaches |
|--------|-----------------|
| [**_config**](domains/_config.md) | Generator opt-in (SQL, Solidity, iLayer) and federation declarations. |
| [**catalog**](domains/catalog.md) | Entity and value types, reference schemes, subtypes, objectification, enums, ring constraints (category hierarchy), and M:N tags. |
| [**orders**](domains/orders.md) | State machines (Order plus Payment), transitions, fact-driven events, derivation rules (order total via sum), subset autofill, and deontic constraints. |
| [**tasks**](domains/tasks.md) | Ternary assignment, priority enums, blocking (acyclic ring), parent-child structure, comments, milestones, and a derived completion percentage. |
| [**content**](domains/content.md) | Symmetric ring (related articles), temporal types, word-count derivation, and a publishing lifecycle. |
| [**scheduling**](domains/scheduling.md) | Objectification with a spanning UC (Booking), recurrence, availability, conflict detection, and temporal derivation. |
| [**notifications**](domains/notifications.md) | Inclusive-or constraint (at least one channel), a delivery state machine with retry, preferences, and deontic permissions. |

## Suggested learning path

If you are new to fact-oriented modeling, read the domains in this order.

1. **catalog** is the simplest domain. It shows how entity types, value types, and constraints fit together. Notice the ring constraint on the category hierarchy.
2. **orders** adds state machines and derivation rules. Watch the `:=` aggregate sum compute the order total from line items automatically.
3. **tasks** brings in a ternary fact type (Assignment) and an acyclic ring (task blocking). Note the difference between irreflexive, asymmetric, and acyclic constraints.
4. **content** shows a symmetric ring constraint (related articles work in both directions).
5. **scheduling** demonstrates objectification with a spanning uniqueness constraint. The Booking entity IS the fact about who booked what when.
6. **notifications** illustrates a disjunctive-mandatory constraint (every user must have at least one delivery channel), plus deontic permissions.

## Features exercised, with pointers

- **Alethic constraints**: `Each Order has exactly one Amount.` (orders)
- **Deontic constraints**: `It is obligatory that each Product has exactly one Price.` (catalog), and `It is forbidden that a Review has Rating less than 1.` (catalog).
- **Derivation rules, aggregate**: `Order has Amount := sum of LineItem Amount where LineItem belongs to that Order.` (orders)
- **Derivation rules, join**: `User accesses Domain := User owns Organization and App belongs to that Organization and Domain belongs to that App.` (metamodel, used via federation)
- **Subset with autofill**: `If some Customer places some Order then that Order has Shipping Address that is that Customer's Shipping Address.` (orders)
- **Compound reference scheme**: `ProductVariant(.id) is an entity type where Product has Size and Color.` (catalog)
- **State machines**: Order has seven statuses and seven transitions (orders).
- **Facts as events**: a fact entering P fires an event the SM consumes (orders, notifications).
- **Federation**: `User is backed by External System 'auth.vin'` is fetched live from the identity provider (_config).
- **Multiple generators**: SQL, Solidity, and iLayer are emitted from the same readings (_config).

## Federation credentials

The tutor declares `User` and `Stripe Customer` as federated. Live fetches need secrets:

```bash
export AREST_SECRET_AUTH_VIN='your-auth-vin-api-key'
export AREST_SECRET_STRIPE='sk_test_...'
```

Without these secrets, the MCP server still loads the schema and accepts writes to local nouns; federated reads return the empty set (the OWA safe default).

## Generators

The tutor opts into SQL (SQLite), Solidity, and iLayer. After compile, each entity has the following emitted artifacts:

- `sql:sqlite:{table}` is a CREATE TABLE with UNIQUE, NOT NULL, and CHECK constraints, plus triggers for derivations.
- `solidity:{Noun}` is a smart contract with a struct, events, a `create` function, and one function per state machine transition.
- `ilayer:{Noun}` is a typed UI layer definition.

To regenerate the Solidity specifically:

```bash
yarn compile
cat .out/Generated.sol
```

To get SQL DDL via the MCP server, ask the agent for "the SQL schema for the orders domain", and the agent will invoke `systemCall("sql:sqlite:order", "")` for each table.

## Writing your own domain

Add a new markdown file under `domains/`. The parser loads every `.md` file in that directory, so your new nouns, fact types, constraints, state machines, and derivation rules become part of the same unified schema.

If you introduce cycles in your derivation rules or contradictions in your constraints, the compiler will reject them by way of Theorem 3: Completeness. If you declare an entity backed by an external system, that entity becomes federated automatically.

## License

MIT
