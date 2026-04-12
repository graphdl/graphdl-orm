# 07 · Generators

arest produces runtime artifacts for several targets from the same readings. Each target is called a **generator** and must be opted in by an instance fact in your readings. A generator that is not opted in produces nothing.

## Opt-in

Declare that an App uses a generator by writing an instance fact:

```forml2
App 'myapp' uses Generator 'sqlite'.
```

A single app can opt into several generators:

```forml2
App 'myapp' uses Generator 'sqlite'.
App 'myapp' uses Generator 'xsd'.
App 'myapp' uses Generator 'ilayer'.
```

The parser reads these before the regular compile and pre-populates the active generator set. Inside the compiler, each generator's code path is gated: `if generators.contains("sqlite") { ... }`.

From the CLI, you can also opt in explicitly:

```bash
arest-cli readings/ --db app.db   # generators read from instance facts
```

Tests that want a specific generator enabled regardless of readings can call `set_active_generators` directly.

## SQL

Seven dialects supported:

- `sqlite`
- `postgresql`
- `mysql`
- `sqlserver`
- `oracle`
- `db2`
- `clickhouse`

Each produces DDL for the RMAP tables derived from your readings. Opting into any SQL dialect also opts the engine into SQL-trigger-based derivation: every derivation rule becomes a `CREATE TRIGGER` statement that materializes derived facts into their own tables.

```forml2
App 'myapp' uses Generator 'postgresql'.
```

Produces defs named `sql:postgresql:{table}` for each RMAP table. Access via:

```bash
arest-cli "sql:postgresql:order" "" --db app.db
```

When a SQL generator is active, the validator only runs non-DDL constraints (Ring, Subset, Equality, Exclusion, deontic, Frequency). UC/MC/VC are enforced by the generated DDL (UNIQUE, NOT NULL, CHECK) and skipped in the engine to avoid double validation.

## iLayer

iLayer is the UI layer format: one def per entity noun describing object type, facts, and transitions. Used by the AREST Next.js / iOS / Android frontends.

```forml2
App 'myapp' uses Generator 'ilayer'.
```

Produces defs named `ilayer:{Noun}` returning a structured object:

```json
{
  "noun": "Order",
  "objectType": "entity",
  "facts": [ ... ],
  "transitions": [ ... ]
}
```

## XSD

XML Schema Definition — generates type definitions for each noun. Useful for SOAP / XML interchange systems.

```forml2
App 'myapp' uses Generator 'xsd'.
```

Produces defs named `xsd:{Noun}`.

## Solidity

Each entity noun becomes an Ethereum smart contract with:

- A typed `struct Data` with RMAP-derived fields
- A `bytes32 status` field for state machine tracking
- One `event` declaration per fact type (facts-as-events)
- An `onlyInStatus` modifier enforcing SM guards
- A `create(...)` function with UC check and initial-status assignment
- One function per SM transition guarded by the modifier

Opt in:

```forml2
App 'myapp' uses Generator 'solidity'.
```

Call `compile_to_solidity(state)` from Rust or use the MCP `compile` tool targeting the solidity generator. The output is `solc`-compilable Solidity 0.8.20.

## Verilog (FPGA)

Each entity noun becomes a synthesizable Verilog module with:

- Clock and reset ports
- Input wires for each RMAP column (e.g. `amount`, `customer_id`)
- An output `valid` register
- A clocked always block

```forml2
App 'myapp' uses Generator 'fpga'.
```

The emitted Verilog is a first-pass module shell. Constraint enforcement and state-machine transitions as synthesizable circuits are future work; the current generator proves the pipeline is in place.

## Test

The `test` generator produces fixture defs useful for exercising the compiled model in unit tests.

```forml2
App 'myapp' uses Generator 'test'.
```

## Multi-target deployment

Because every generator produces named defs, you can invoke them selectively at runtime. A single compile can produce SQL for the primary database, Solidity for on-chain settlement, and XSD for a SOAP partner:

```forml2
App 'enterprise' uses Generator 'postgresql'.
App 'enterprise' uses Generator 'solidity'.
App 'enterprise' uses Generator 'xsd'.
```

Each downstream consumer calls the def that interests them; the others are ignored.

## Writing a new generator

A generator is a function with the signature:

```rust
pub fn compile_to_foo(state: &Object) -> String
```

or a set of defs pushed during `compile_to_defs_state`. The conventions:

- Read schema information from the state or the reconstructed domain.
- Use RMAP tables (`rmap::rmap(&domain)`) for typed output.
- Gate with `generators.contains("foo")` so the generator is only active when opted in.
- Prefer pure FP style — iterator combinators, no mutable accumulators unless the output is a single string.

See `crates/arest/src/generators/solidity.rs` or `fpga.rs` for a current example.

## What's next

Your readings now produce running applications across many runtimes. [Federation](08-federation.md) shows how to bring in data from systems you do not own.
