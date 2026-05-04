# AREST Tutor

This is a teaching app that exercises every AREST framework concept. Each domain demonstrates specific capabilities. Read the readings, run the MCP server, poke at the system from Claude Code, and build intuition for how fact-oriented modeling produces correct-by-construction applications.

## Quick start

The tutor ships as a subfolder of the main AREST repo, so one clone covers everything:

```bash
git clone https://github.com/graphdl/arest
cd arest/tutor
yarn install
yarn compile   # round-trips every domain through the Rust compiler
```

With the `arest` MCP connected in Claude Code, ask the agent things like:

> "Create an order for customer alice@example.com, then place it and ship it."
> "Why was this order marked delivered?"
> "Show me every category that is a descendant of Electronics."

Every response is a ρ-application over the facts. No handler code was written; the readings produced the behavior.

## Using the tutor from Claude Code or Claude Desktop

The tutor is built into the main `arest` MCP server — you do **not** need
a separate `arest-tutor` registration. Any current `arest` registration
exposes the lesson tool plus a fully isolated tutor sandbox:

| Tool                                    | Purpose                                                                  |
|-----------------------------------------|--------------------------------------------------------------------------|
| `tutor`                                 | List lessons or load one (with auto-graded `~~~ expect` predicate).      |
| `tutor.list` / `tutor.get` / `tutor.query` | Read against the sandbox.                                              |
| `tutor.apply` / `tutor.compile` / `tutor.propose` / `tutor.actions` | Mutate the sandbox.                       |
| `tutor.reset`                           | Wipe the sandbox and re-bootstrap from `tutor/domains/`.                 |

The sandbox is a separate `D` (state) loaded from `tutor/domains/`; your
active app is **never touched** by `tutor.*` calls.

### Sandbox persistence

In CLI-DB mode (when `AREST_CLI` is set, the production setup), the
sandbox is backed by its own SQLite file:

- Default path: `tutor/.sandbox/tutor.db` (gitignored, per-developer).
- Override: set `$AREST_TUTOR_DB` to any path, e.g. a sync-friendly
  location if you want lesson progress to follow you across machines.
- `tutor.reset` deletes this file and re-bootstraps from
  `tutor/domains/` on the next call.

In WASM mode (no `AREST_CLI`), the sandbox lives only in process memory
and is lost on MCP restart — fine for quick experimentation; CLI mode is
the right choice for working through lessons in order.

### Migrating from `arest-tutor`

The previous `arest-tutor` named entry in `.mcp.json` has been removed.
If your personal config still has it, the entry continues to work but is
redundant — point Claude at `arest` instead.

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
2. **orders** adds state machines and derivation rules. Watch the `*` iff aggregate sum compute the order total from line items automatically.
3. **tasks** brings in a ternary fact type (Assignment) and an acyclic ring (task blocking). Note the difference between irreflexive, asymmetric, and acyclic constraints.
4. **content** shows a symmetric ring constraint (related articles work in both directions).
5. **scheduling** demonstrates objectification with a spanning uniqueness constraint. The Booking entity IS the fact about who booked what when.
6. **notifications** illustrates a disjunctive-mandatory constraint (every user must have at least one delivery channel), plus deontic permissions.

## Features exercised, with pointers

- **Alethic constraints**: `Each Order has exactly one Amount.` (orders)
- **Deontic constraints**: `It is obligatory that each Product has exactly one Price.` (catalog), and `It is forbidden that a Review has Rating less than 1.` (catalog).
- **Derivation rules, aggregate**: `* Order has Amount iff Amount is the sum of LineItem Amount where some LineItem belongs to that Order.` (orders)
- **Derivation rules, join**: `+ User accesses Domain if User owns Organization and App belongs to that Organization and Domain belongs to that App.` (metamodel, used via federation)
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
