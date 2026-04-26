# AREST CLI — Getting Started

`arest-cli` runs the engine on your machine. Point it at a directory of
FORML 2 readings; it compiles them, persists state to SQLite, and exposes
every SYSTEM call as a single command shape.

There is no separate "compile", "synthesize", or "forward-chain" subcommand.
Everything is `SYSTEM:x = ⟨o, D'⟩`, exposed as `arest-cli <key> <input>`.

## Prerequisites

- Rust nightly (the engine uses some unstable features for the WASM target;
  the CLI itself is stable but the workspace pins nightly):

```bash
curl https://sh.rustup.rs -sSf | sh -s -- --default-toolchain nightly -y
```

- For the SQLite backend: nothing extra (the `local` feature bundles SQLite
  via `rusqlite`).

## Build

```bash
git clone https://github.com/graphdl/arest
cd arest
cargo build --release --bin arest-cli --features local
# Binary at target/release/arest-cli (or .exe on Windows)
```

Add it to your `PATH`, or symlink it:

```bash
ln -s "$PWD/target/release/arest-cli" /usr/local/bin/arest-cli
```

## Compile readings into a database

```bash
arest-cli readings/ --db app.db
```

This walks `readings/`, parses every `*.md`, runs the full Stage-1 + Stage-2
compile pipeline, and writes the resulting state to `app.db`. Pass multiple
directories to merge them:

```bash
arest-cli readings/core readings/myapp --db app.db
```

## Issue SYSTEM calls

Once `app.db` exists, every other invocation is `<key> <input>`:

```bash
# Create
arest-cli --db app.db "create:Order" "<<Order Id, ord-1>, <Customer, acme>>"

# Transition (state machine)
arest-cli --db app.db "transition:Order" "<ord-1, place>"

# Read with HATEOAS links
arest-cli --db app.db "get:Order" "ord-1"

# List
arest-cli --db app.db "list:Order" ""

# Query a fact type
arest-cli --db app.db "query:Order_was_placed_by_Customer" '{"Customer": "acme"}'

# Update
arest-cli --db app.db "update:Order" '{"id": "ord-1", "notes": "rush"}'

# Delete
arest-cli --db app.db "delete:Order" "ord-1"
```

The output is JSON — pipe through `jq` for pretty printing.

## Use the checker

```bash
# Run the full deontic + alethic check against the current state
arest-cli --db app.db "verify" ""

# Check arbitrary text (extracts facts, checks them against constraints)
arest-cli --db app.db "validate" "Alice placed ord-2 on 2026-04-26."

# Explain a single entity (which constraints apply, which fired)
arest-cli --db app.db "explain" '{"noun": "Order", "id": "ord-1"}'

# List legal next actions on a cell (Theorem 5 surface)
arest-cli --db app.db "actions" '{"noun": "Order", "id": "ord-1"}'
```

## Reload after editing readings

```bash
# Re-compile in place; preserves the population
arest-cli readings/ --db app.db

# Or load a single reading at runtime (governed by the deontic gate)
arest-cli --db app.db "load_reading:my-app" "$(cat readings/myapp/orders.md)"
```

## Run from a fresh checkout (no install)

```bash
cargo run --release --bin arest-cli --features local -- readings/ --db app.db
cargo run --release --bin arest-cli --features local -- --db app.db "list:Order" ""
```

## Tests

```bash
cargo test -p arest --lib                     # 940+ engine tests, ~30 s
cargo test -p arest --lib --features parallel # opt-in Rayon for α/Filter
cargo t                                       # alias: dev-fast profile, no opts
```

## Where next

- `docs/02-writing-readings.md` — FORML 2 syntax reference
- `docs/03-constraints.md` — UC / MC / RC / XO / XC / OR / SS / EQ
- `docs/04-state-machines.md` — transitions as derivations
- `tutor/` — 17 lessons across three difficulty tracks
- [`docs/cloud.md`](cloud.md) — same engine, deployed
- [`docs/mcp.md`](mcp.md) — expose the local DB to an AI agent
