# graphdl-orm

An implementation of the [AREST whitepaper](AREST.tex) — *Compiling Facts into Applications*.

Domain knowledge expressed as FORML 2 readings compiles into executable applications via Backus's FFP representation function. The system is a tuple `(O, ρ, D, P, sub)` where readings are FFP objects, ρ maps them to executables, state D is a sequence of cells, population P is Codd's named set, and sub dispatches inputs to state transitions producing REST representations with HATEOAS links.

Based on the work of [John Backus](https://en.wikipedia.org/wiki/John_Backus) (FFP/AST, 1978), [E.F. Codd](https://en.wikipedia.org/wiki/Edgar_F._Codd) (relational model, 1970), [Terry Halpin](https://en.wikipedia.org/wiki/Terry_Halpin) (ORM 2, 2008), and [Roy Fielding](https://en.wikipedia.org/wiki/Roy_Fielding) (REST, 2000).

## Architecture

```
readings (FORML 2)
    │
    ├─► parse_readings_wasm (Rust/WASM)  ──► entities (cells in D)
    │
    ├─► load_ir (compile schema from cells)
    │
    ├─► apply_command (create = emit ∘ validate ∘ derive ∘ resolve)
    │
    └─► representation (entity data + HATEOAS links + _view metadata)
```

Two Durable Objects, matching the paper exactly:

| Component | Paper | Implementation |
|-----------|-------|----------------|
| **D** (state) | Sequence of cells `⟨CELL, n, c⟩` | EntityDB — one DO per entity. `↑n` = get(), `↓n` = put(). |
| **P** (population) | `↑FILE:D` — named set of relations | RegistryDB — population index. Maps (type, id) to cells. |
| **ρ** (representation function) | Maps FFP objects to executables | Rust/WASM engine (arest). Compiles readings, evaluates constraints, forward-chains derivations. |
| **sub** (subsystem dispatch) | Routes inputs to state transitions | itty-router. Dispatches create/update/query/transition/load commands. |

13 TypeScript source files. The complexity lives in ρ (WASM), not in the host.

## Domains and Apps

**Domains are NORMA tabs, not partitions.** A domain organizes discourse — which readings are visible in a tab. Fact types are idempotent: "Customer has Name" declared in both "sales" and "support" is the SAME fact type. All domains compile to a single Universe of Discourse.

**Apps lasso domains into databases.** An App determines which domains are navigable and which users can access them. The Support Tickets app navigates the "support" domain. The admin view shows all domains.

```
App "Support Tickets"
  └── navigable domains: ["support"]
       └── entity types: Support Request, Support Response, Message, ...

App "Admin" (all domains)
  └── navigable domains: ["core", "state", "support", "stripe", ...]
       └── entity types: Noun, Reading, Constraint, Graph Schema, ...
```

Access control is expressed as readings in `organizations.md`:
```
User accesses Domain iff User has Org Role in Organization
  and Domain belongs to that Organization.
User accesses Domain if Domain has Visibility 'public'.
```

The engine evaluates these derivation rules — no procedural access control code.

## Writing Readings

Readings are FORML 2 sentences — natural language with unambiguous grammar (Theorem 1).

```markdown
# Support

## Entity Types

Support Request(.Request Id) is an entity type.
Category is a value type.
  The possible values of Category are 'question', 'feature-request', 'incident'.

## Fact Types

Support Request has Subject.
  Each Support Request has exactly one Subject.
Support Request has Category.
  Each Support Request has at most one Category.

## Constraints

It is obligatory that each Support Response is professional.

## State Machine

State Machine Definition 'Support Request' is for Noun 'Support Request'.
Status 'Received' is initial in State Machine Definition 'Support Request'.
Transition 'categorize' is from Status 'Received'.
  Transition 'categorize' is to Status 'Categorized'.
```

Abstract nouns prevent direct instantiation:
```
Request is abstract.
```
Or via totality: `Each Request is a Support Request or a Feature Request.`

Order and indentation do not matter. Each sentence has exactly one parse.

## API

```
# Entity CRUD (cells in D)
POST   /api/entities/:type              — create (emit ∘ validate ∘ derive ∘ resolve)
GET    /api/entities/:type              — list (requires ?domain=)
GET    /api/entities/:type/:id          — get (includes _view metadata + HATEOAS links)
PATCH  /api/entities/:type/:id          — update (↓n with merged data)
DELETE /api/entities/:type/:id          — delete (hard delete from population)

# State Machine Transitions
GET    /api/entities/:type/:id/transitions  — available transitions (Theorem 3: links from P)
POST   /api/entities/:type/:id/transition   — fire transition (state machine fold)

# Query (θ₁ operations on P)
GET    /api/query?q=...&domain=...      — prove goal via backward chaining

# Seed (parse readings via ρ → cells in D)
POST   /api/seed                        — parse FORML 2 markdown via WASM → materialize cells
GET    /api/seed                        — population stats
DELETE /api/seed                        — wipe population

# Access (derived from readings)
GET    /api/access                      — user's orgs, apps, domains (derivation rules)

# Evaluation (ρ applied to P)
POST   /api/evaluate                    — constraint evaluation via WASM
POST   /api/synthesize                  — noun knowledge synthesis
```

Self-describing representations include `_view` (view metadata derived from readings) and `_nav` (navigation context from access derivation rules):

```json
{
  "docs": [...],
  "_view": { "type": "ListView", "title": "Support Request", "fields": [...], "constraints": [...] },
  "_nav": { "domains": [...], "apps": [...], "breadcrumb": ["support", "Support Request"] },
  "_links": { "self": "...", "collection": "...", "create": "..." }
}
```

## Development

```bash
npm install
npm run dev          # wrangler dev
npm test             # vitest (67 tests)
npx tsc --noEmit     # type check

# Rust/WASM engine
cd crates/arest
cargo test           # 236+ tests
wasm-pack build --target web --out-dir pkg

# Deploy
npx wrangler deploy

# Seed
curl -X POST https://api.auto.dev/api/seed -F "core=@readings/core.md" -F "support=@readings/support.md"
```

## Theorems

The [whitepaper](AREST.tex) proves five properties:

1. **Grammar Unambiguity** — each FORML 2 sentence has exactly one parse
2. **Specification Equivalence** — `parse⁻¹ ∘ compile⁻¹ ∘ compile ∘ parse = id`
3. **Completeness of State Transfer** — create produces entity + state machine + derived facts + violations + HATEOAS links
4. **HATEOAS as Projection** — links are θ₁ projections of transition facts in P
5. **Derivability** — every domain value is `(ρf):P` for some `f ∈ O`

## License

MIT
