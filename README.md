# graphdl-orm

An implementation of [AREST](AREST.tex) — *Compiling Facts into Applications*.

FORML 2 readings compile to named lambda functions via Backus's FFP representation function. SYSTEM is the only function. Every operation — compile, create, validate, transition, query — is a def in D resolved via rho. The engine is a beta-reducer targeting WASM (Cloudflare Workers) and eventually FPGA.

Based on [John Backus](https://en.wikipedia.org/wiki/John_Backus) (FFP/AST, 1978), [E.F. Codd](https://en.wikipedia.org/wiki/Edgar_F._Codd) (relational model, 1970), [Terry Halpin](https://en.wikipedia.org/wiki/Terry_Halpin) (ORM 2, 2008), and [Roy Fielding](https://en.wikipedia.org/wiki/Roy_Fielding) (REST, 2000).

## CLI

```bash
# Compile readings into SQLite
arest readings/ --db app.db

# Single SYSTEM call
arest "transitions:Case" "Open" --db app.db

# Create entity with fact pairs
arest "create:Order" "<<id, ord-1>, <customer, acme>>" --db app.db

# REPL
arest --db app.db

# Flags
arest readings/ --db app.db --strict        # reject undeclared nouns
arest readings/ --db app.db --no-validate   # skip constraint validation
```

## WASM API

One export. SYSTEM is the only function.

```
system(handle, key, input) -> string  — SYSTEM:x = (rho(entity(x):D)):op(x)
```

All operations dispatch via rho from defs in D. On FPGA, each def is a synthesized circuit.

```javascript
system(h, 'compile', domainReadings)          // self-modification
system(h, 'transitions:Order', 'In Cart')     // SM query
system(h, 'create:Order', '<<id, ord-1>>')    // create with fact pairs
system(h, 'debug', '')                        // state projection
```

## Architecture

```
readings (FORML 2 text)
    |
    compile (Platform primitive)  --> facts in P, defs in DEFS
    |
    system(h, key, input)         --> rho-dispatch --> beta-reduce --> D'
```

| Paper | Implementation |
|-------|----------------|
| D (state) | Sequence of cells. EntityDB (one Durable Object per entity). |
| P (population) | Named set of elementary facts. RegistryDB (population index). |
| rho | Rust/WASM engine. Compiles readings, evaluates constraints, forward-chains derivations. |
| SYSTEM | `system_impl`: one line of rho-dispatch + state transition. No routing. |
| Platform primitives | `Func::Platform("compile")`, `Func::Platform("create:Order")`, `Func::Platform("transition:Order")`. Synthesizable gates. |

## Readings

FORML 2 sentences with unambiguous grammar (Theorem 1).

```
Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Order was placed by Customer.
  Each Order was placed by exactly one Customer.

State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
```

Constraints, state machines, derivation rules, and instance facts are all readings. The compiler recognizes a single object that occupies all four roles (Theorem 2).

## REST API

AREST routes derive navigation from the constraint graph (Theorem 4):

```
GET  /arest/                              root resource
GET  /arest/:collection                   list with _links + _schema
GET  /arest/:collection/:id              entity with HATEOAS links
```

Write operations go through the apply command (Eq. 10: create = emit . validate . derive . resolve):

```
POST /api/parse                           compile . parse (readings -> D)
POST /api/entities/:noun                  create entity
POST /api/entities/:noun/:id/transition   fire state machine event
```

## Security

Authorization is a constraint in the readings, not middleware. Identity (`sender`) is pushed as a User fact during resolve; alethic constraints enforce access. Signing via HMAC-SHA256 (`AREST_HMAC_KEY` env var). SSRF denylist on External System URLs. Metamodel noun namespace protection. Input bounds are platform gates (hardware buffer size). Audit trail in `audit_log` cell.

## MCP Server

```json
{
  "mcpServers": {
    "graphdl": {
      "command": "npx",
      "args": ["-y", "graphdl-orm", "mcp"],
      "env": { "GRAPHDL_MODE": "local", "GRAPHDL_READINGS_DIR": "./readings" }
    }
  }
}
```

Tools: `graphdl_create`, `graphdl_transition`, `graphdl_compile`, `graphdl_list`, `graphdl_get`, `graphdl_schema`, `graphdl_evaluate`, `graphdl_audit_log`, `graphdl_verify_signature`.

Prompts: `graphdl_overview`, `graphdl_entity_modeling`, `graphdl_advanced_constraints`, `graphdl_derivation_deontic`, `graphdl_verbalization`, `graphdl_principles`, `graphdl_api`.

## Development

```bash
# Rust engine
cd crates/arest
cargo test --features local    # 851 tests
cargo build --release --features local

# WASM
cargo build --target wasm32-unknown-unknown --no-default-features --features cloudflare
cargo build --target wasm32-wasip2 --no-default-features --features wit

# TypeScript
yarn install
yarn test               # vitest
npx tsc --noEmit        # type check
```

## Theorems

The [whitepaper](AREST.pdf) proves five properties:

1. **Grammar Unambiguity** — each FORML 2 sentence has exactly one parse
2. **Specification Equivalence** — parse and compile are injective; the reading IS the executable
3. **Completeness of State Transfer** — create reaches the least fixed point with all violations
4. **HATEOAS as Projection** — all links are theta-1 operations on P and S
5. **Derivability** — every value in the representation is a rho-application

Self-modification preserves all five (Corollary: Closure). Constraint consensus enables peer-to-peer validation without external protocol (Corollary: Consensus).

## License

MIT
