# graphdl-orm

An implementation of [AREST](AREST.tex) — *Compiling Facts into Applications*.

FORML 2 readings compile to named lambda functions via Backus's FFP representation function. SYSTEM is the only function. Every operation — compile, create, validate, transition, query — is a def in D resolved via rho. The engine is a beta-reducer targeting WASM (Cloudflare Workers) and eventually FPGA.

Based on [John Backus](https://en.wikipedia.org/wiki/John_Backus) (FFP/AST, 1978), [E.F. Codd](https://en.wikipedia.org/wiki/Edgar_F._Codd) (relational model, 1970), [Terry Halpin](https://en.wikipedia.org/wiki/Terry_Halpin) (ORM 2, 2008), and [Roy Fielding](https://en.wikipedia.org/wiki/Roy_Fielding) (REST, 2000).

## WASM API

Two exports. SYSTEM is the only function.

```
create()                              — allocate empty D with platform primitives
system(handle, key, input) -> string  — SYSTEM:x = (rho(entity(x):D)):op(x)
```

Self-modification: `system(h, 'compile', readings_text)` ingests readings. All other operations dispatch via rho from defs in D. No match arms, no if-branches. On FPGA, each def is a synthesized circuit.

```javascript
const h = create()
system(h, 'compile', coreReadings)
system(h, 'compile', stateReadings)
system(h, 'compile', domainReadings)
system(h, 'transitions:Order', 'In Cart')  // rho-dispatch
system(h, 'debug', '')                      // rho-dispatch
system(h, 'apply', JSON.stringify({...}))   // create pipeline
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
| Platform primitives | `Func::Platform("compile")`, `Func::Platform("apply_command")`. Synthesizable gates. |

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

Authorization is a constraint in the readings, not middleware. The compile operation is gated: after bootstrap, bare compile is rejected. Noun namespace protection via UC on Object Type. Input bounds are platform gates (hardware buffer size). The evolution domain's state machine governs schema modification.

See `src/tests/security/authorization.test.ts` for the threat model.

## Development

```bash
yarn install
yarn dev                # wrangler dev
yarn test               # vitest (265 tests)
yarn build:wasm         # Rust -> WASM
npx tsc --noEmit        # type check

# Rust engine
cd crates/arest
cargo test              # 575 tests
```

## Theorems

The [whitepaper](AREST.tex) proves five properties:

1. **Grammar Unambiguity** — each FORML 2 sentence has exactly one parse
2. **Specification Equivalence** — parse and compile are injective; the reading IS the executable
3. **Completeness of State Transfer** — create reaches the least fixed point with all violations
4. **HATEOAS as Projection** — all links are theta-1 operations on P and S
5. **Derivability** — every value in the representation is a rho-application

Self-modification preserves all five (Corollary: Closure). Constraint consensus enables peer-to-peer validation without external protocol (Corollary: Consensus).

## License

MIT
