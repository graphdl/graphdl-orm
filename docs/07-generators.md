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

## OpenAPI

OpenAPI 3.1 is the interchange format for REST clients — Swagger/Redoc viewers, typed client generators (openapi-typescript, openapi-generator), and IDE autocompletion. The generator derives the entire document from RMAP output plus the metamodel's fact types and state machines. No hand-written schema, no drift.

Opt in:

```forml2
App 'myapp' uses Generator 'openapi'.
```

Produces one def per opted-in App, keyed `openapi:{snake(app-slug)}`. Exposed publicly at:

```
GET /api/openapi.json?app={app-slug}
```

### What the document contains

**`components.schemas`** — one entry per entity noun in the compile, plus a shared `Violation` component. Each entity's schema is built from `rmap::rmap(domain)`:

- Columns become `properties`.
- `!nullable` columns become `required`.
- Columns with a `references` target emit `$ref: "#/components/schemas/{Target}"` instead of a scalar type.
- SQL column types map to JSON Schema scalar types (`INTEGER`/`BIGINT`/`SMALLINT` → `integer`, `REAL`/`NUMERIC`/`DECIMAL` → `number`, `BOOLEAN` → `boolean`, everything else → `string`).
- Value types with declared enum values add `enum` to the property.
- State machines contribute a `status` property whose enum is the declared status set (read-side projection of `Status is defined in State Machine Definition` — storage is the RMAP column, behaviour lives in the SM).

**`paths`** — two-to-four routes per entity noun per Theorem 4 (HATEOAS as Projection):

Always emitted:

- `GET  /{plural}`         — list, with `Filter(p_live):P` server-side per Corollary 2 (deletion-as-terminal-state).
- `POST /{plural}`         — create. Request body `$ref`s the noun schema.
- `GET  /{plural}/{id}`    — read.
- `PATCH /{plural}/{id}`   — update.

No `DELETE`: per AREST §4.1 and Corollary 2, deletion is a transition to a terminal status, not an erase. The list endpoint filters terminal entities out server-side.

Emitted only when the noun has a State Machine Definition (Theorem 4a — transition links as θ₁ projection):

- `POST /{plural}/{id}/transition`   — fire an event. Request body enumerates every declared transition event; the server no-ops on events invalid from the entity's current status (paper §"machine fold").
- `GET  /{plural}/{id}/transitions`  — available events from the current status.

Emitted per binary fact type `f` the noun participates in (Theorem 4b navigation):

- `GET  /{plural}/{id}/{other-plural}` — related collection of the co-participating noun. Ring fact types and multiple binaries between the same noun pair are disambiguated by a verb-derived slug.

### Plural slug resolution

The path slug defaults to `snake(noun) + "s"` — fine for regular English plurals ("Organization" → "organizations"). Irregulars are declared explicitly:

```forml2
Noun 'Policy' has Plural 'policies'.
Noun 'Category' has Plural 'categories'.
Noun 'Child' has Plural 'children'.
```

The generator prefers an explicit `Noun has Plural` instance fact over the fallback — facts-all-the-way-down, no dedicated struct field.

### Response envelope — Theorem 5

Every operation's response declares the same four-key envelope per the paper's `repr(e, P, S)`:

```json
{
  "data": <noun-row or array of noun-rows>,
  "derived": { "<rule-name>": <value>, ... },
  "violations": [ <Violation>, ... ],
  "_links": {
    "transitions": [{"event": "...", "href": "...", "method": "POST"}, ...],
    "navigation": {"<relation>": "<uri>", ...}
  }
}
```

- `data` — the 3NF row (item responses) or an array of rows (list responses). `$ref` to the noun's component schema.
- `derived` — derivation-rule outputs for this entity. Only on single-entity reads; `additionalProperties: true` so new rules surface without regenerating clients.
- `violations` — array of `Violation` objects. The `reading` field carries the original FORML 2 sentence verbatim per Corollary 1 — clients can surface the reading to users as an explanation.
- `_links` — Theorem 4's `links_full(e, n, status(e, P))`: valid transitions plus the navigation URIs.

`data` and `_links` are required; `derived` and `violations` are optional because not every response carries them (paginated list pages often carry neither).

### Violation schema

Declared unconditionally under `components.schemas.Violation`:

```json
{
  "type": "object",
  "properties": {
    "reading":      { "type": "string" },   // original FORML2 per Cor 1
    "constraintId": { "type": "string" },
    "modality":     { "type": "string", "enum": ["alethic", "deontic"] },
    "detail":       { "type": "string" }
  },
  "required": ["reading", "constraintId", "modality"]
}
```

When an App's readings load `readings/outcomes.md`, the user's own RMAP-derived `Violation` schema overrides this default — first insertion wins.

### Live updates

Every OpenAPI response represents the state at the moment of the request. For long-lived UIs that want to react to changes, subscribe to the event stream alongside:

```
GET /api/events?domain={domain}&noun={noun}&entityId={entityId}
```

Every field is optional (except `domain`) — narrower filters receive fewer events. Server-Sent Events frames carry the `CellEvent` JSON (one event per matching mutation). Wire this to TanStack Query cache invalidation, or a Redux middleware, or an EventSource directly.

Post-mutation hooks in the entity CRUD handlers fire a publish for every committed create/delete; the transition write path fires a `transition` event too. See `src/broadcast-do.ts`.

## XSD

XSD generates an XML Schema Definition with type definitions for each noun. It is useful for SOAP and XML interchange systems.

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

- Clock and reset ports (`clk`, `rst_n`)
- Input wires for each RMAP column (e.g. `amount`, `customer_id`)
- An output `valid` register
- A clocked always block

```forml2
App 'myapp' uses Generator 'fpga'.
```

### Top module

Alongside the per-entity modules, the generator emits a `top` module that wires them together so the output is a self-contained buildable unit:

```verilog
module top (
    input wire clk,
    input wire rst_n,
    output reg all_valid
);
    wire order_valid;
    wire customer_valid;

    order    order_inst    (.clk(clk), .rst_n(rst_n),
                            .id_in({256{1'b0}}), .valid(order_valid));
    customer customer_inst (.clk(clk), .rst_n(rst_n),
                            .name_in({256{1'b0}}), .valid(customer_valid));

    always @(posedge clk) begin
        all_valid <= rst_n & order_valid & customer_valid;
    end
endmodule
```

`clk` and `rst_n` fan out to every entity; each entity's `valid` is AND-reduced into a single system-level `all_valid`. Column inputs are tied to `{256{1'b0}}` so synthesis tools do not flag unconnected input ports — integrators replace those zero drivers with real wires (BRAM ports, pipeline stages) when integrating with storage. If the compile has no entity nouns, `top` is elided entirely — no entities, nothing to wire, just the header line.

### Constraint modules

Every `Constraint` cell entry compiles to one Verilog module whose `violation` output AND-reduces (after inversion) into `top`'s `constraint_ok` signal. The following predicates now emit real hardware:

- **UC (Uniqueness).** Pairwise comparator tree over a flattened row bus. `violation = 1` iff two active rows match on the spanned columns. DEPTH and ROW_WIDTH are `parameter`s so synthesis can tune the tree; top ties `rows_flat` / `row_count` to zero by default — integrators rewire to the entity BRAM's `rdata_a` ports.
- **MC (Mandatory).** Per-row zero-sentinel compare. `violation = 1` iff any active row holds all zeros on the payload.
- **FC (Frequency).** Saturating counter with `create_pulse` / `terminate_pulse` inputs; `violation` high whenever the live count sits outside the parsed `[MIN_COUNT, MAX_COUNT]` window. Bounds come from the Constraint's `text` binding (legacy `at most N and at least M` pattern). Integrators wire the pulses to the SM module's create / terminate edges.
- **Ring — IR / AS / AT / SY / IT / TR / AC / RF.** Pairwise predicates over `(left, right)` row halves. IR checks for self-loops, AS / AT check for inverse pairs (with / without the self-loop allowance), SY / RF check for missing inverses / self-loops, IT / TR check the transitive-closure predicates, AC approximates acyclicity via the two-row cycle case.
- **VC (Value).** Case comparator against the declared enum-value set (read from the `EnumValues` cell and baked as 256-bit ASCII literals). `violation = 1` iff some active row matches none of the declared values.

### What's still future work

The cross-fact-type constraints — **SS (Subset), EQ (Equality), XC (Exclusion), OR (Inclusive-Or), XO (Exclusive-Or)** — emit canonical-shape stub modules with an explicit `// TODO` comment and `violation` tied to zero. Real predicates require a two-bank BRAM handshake (two distinct entity BRAMs read simultaneously) and land after the single-bank handshake for UC/MC/ring/VC stabilises.

State-machine transition signalling is in place (see `emit_sm_modules`), but the pulse bridge that wires create / terminate transitions into FC's counter pulses is still an integrator step — the ports are there; the default tie-off is `1'b0`.

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
- Prefer a pure functional style with iterator combinators. Avoid mutable accumulators unless the output is a single string.

See `crates/arest/src/generators/solidity.rs` or `fpga.rs` for a current example.

## What's next

Your readings now produce running applications across many runtimes. [Federation](08-federation.md) shows how to bring in data from systems you do not own.
