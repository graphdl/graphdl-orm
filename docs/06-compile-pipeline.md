# 06 · The Compile Pipeline

This doc walks through what happens between the moment you hand the engine a directory of readings and the moment it is ready to answer queries. You do not need to understand every step to use arest, but understanding the pipeline helps when you debug, optimize, or extend the system.

## One-line summary

```
readings text → parse → Domain → domain_to_state → P (Map) → compile_to_defs_state → defs (Vec) → defs_to_state → D (Map) → split into cells
```

`P` is the population (facts). `D` is the state, meaning `P` plus all compiled defs. At rest, `D` is held as a `HashMap<String, Arc<RwLock<Object>>>` — one independently-lockable cell per key, with Backus's fetch / store operators mapped onto per-cell read and write locks. Every MCP call operates against that cells map through a snapshot / diff / commit cycle (see "Concurrency" below).

## Step 1: Parse

`parse_markdown` reads each reading and classifies it into one of three families (Theorem 1: Grammar Unambiguity):

- Quantified constraints (`Each X has some Y`, `For each X, at most one Y ...`)
- Conditional constraints (`If ... then ...`, `... iff ...`)
- Multi-clause constraints (`exactly one of the following holds ...`)

Nouns are matched longest-first so that multi-word names like `State Machine Definition` are recognized before `State` alone. Unknown nouns are auto-created in permissive mode, or rejected in `--strict` mode.

The parser produces a `Domain` struct containing:

- `nouns`: the entity and value type declarations.
- `fact_types`: every declared fact type along with its roles.
- `constraints`: every constraint with its spans and modality.
- `state_machines`: SM definitions and their transitions.
- `derivation_rules`: rules with resolved antecedents and consequents.
- `general_instance_facts`: instance facts and their subject, object, and field bindings.

Parse is the slowest step in compile, but it only runs once per compile — per-command `create` does not parse, since it reads already-compiled defs instead.

## Step 2: `domain_to_state`

The `Domain` struct becomes a single `Object::Map` where each cell holds a sequence of facts. There is one cell per category (Noun, FactType, Constraint, DerivationRule, and so on) plus one cell per declared fact type ID for instance facts.

This step is O(n) in the number of facts. Cells are built mutably and wrapped in a `Map` once at the end, which is cheaper than the equivalent fold over `cell_push` would be (that would be O(n²)).

## Step 3: `compile_to_defs_state`

This is the big one. It reconstructs a Domain from the state (a round trip, since the engine operates over the Object representation rather than the struct), then produces a flat list of `(name, Func)` pairs.

- **Constraints.** Each constraint gets `constraint:{id}` plus `validate:{fact_type_id}` per-FT indexed validators.
- **Machines.** Each SM gets `machine:{noun}`, `machine:{noun}:initial`, and `transitions:{noun}`.
- **Derivations.** Each user-defined and synthetic derivation (CWA negation, subtype inheritance, transitivity) gets `derivation:{id}`.
- **Derivation index.** The compiler emits `derivation_index:{noun}` cells so that `create` can gate which rules run.
- **Shard map.** The compiler emits `shard:{fact_type_id}` mapping each fact type to its owning cell (the RMAP partition).
- **Schemas.** Each fact type gets `schema:{fact_type_id}` Construction funcs (tuple constructors).
- **Resolve.** Each noun gets `resolve:{noun}`, a condition chain mapping field name to fact type.
- **Query.** Each fact type gets `query:{fact_type_id}` returning role metadata.
- **Populate.** Each federated noun gets `populate:{noun}`.
- **Generators.** Any generator whose opt-in appears as an instance fact emits `sql:sqlite:{table}`, `xsd:{noun}`, `ilayer:{noun}`, `test:{id}`, and similar keys.

The result is a vector of named Funcs. Debug output during compile shows timing per phase.

## Step 4: `defs_to_state`

This step merges the compiled defs with the existing state cells into a single `Object::Map`. Every def becomes a key; every cell becomes a key; lookup is O(1). The result is the `D` that the rest of the runtime consumes.

## RMAP: relational mapping

Halpin Ch. 10 gives the procedure. The engine's RMAP runs as follows.

1. Binarize exclusive unaries (XO unary fact types become a status column on the entity).
2. Absorb subtypes. The compiler chooses the partitioned strategy when the subtype has its own fact types and the single-table strategy otherwise.
3. Classify fact types by UC arity:
   - **Compound UC** (spanning ≥ 2 roles) becomes its own M:N table.
   - **Single-role UC** is functional; the compiler absorbs it into the entity's table, with the other role as a column.
   - **No UC** becomes a junction table.
4. Apply 1:1 absorption. When both roles have single-role UCs, the compiler absorbs into the mandatory side; if neither is mandatory, it absorbs into the entity-over-value side or the larger table.
5. Compound reference schemes become a composite primary key on the components.
6. Constraints map directly: UC becomes UNIQUE, MC becomes NOT NULL, VC becomes CHECK, and SS becomes a foreign key.

The result is a list of `TableDef` structs with columns, primary keys, uniqueness constraints, and check constraints. SQL and FPGA generators both consume this same structure.

## Hot paths

On every `create`:

1. `resolve` produces identity from the ref scheme. Runtime functions (federation, external calls) execute here.
2. `derive` forward-chains the relevant derivation rules to the least fixed point.
3. `validate` applies every constraint as a restriction over P. The compiler gates by fact type when possible.
4. The SM fold consumes the events generated by resolve and derive, then advances the status.
5. `emit` constructs the representation `⟨P', V, links⟩`.

Constraints are indexed: `validate:{fact_type_id}` runs only the constraints that span that FT. The full `validate` is only needed when the engine does not know which FT changed.

Derivations are indexed by noun: `derivation_index:{Order}` lists the rule IDs relevant to Orders. The engine only forward-chains those.

Fetches against `D` are O(1) (backed by `HashMap`). At realistic def counts the fetch cost is negligible relative to the work inside the ρ-application itself.

## Incremental compile

`compile` can be called on a running system to add new readings (Corollary 5: Closure Under Self-Modification). The new definitions merge into `DEFS` via `↓DEFS`, and subsequent `SYSTEM` applications see them. This is how `propose` eventually lands: a Domain Change transitioning to Applied invokes `compile` on the proposed readings.

## Metamodel cache

The metamodel (`readings/core.md` and the rest of the bundled domains) does not change between tenants. It compiles once per process: the first call through `OnceLock` runs parse + `compile_to_defs_state` over the merged metamodel and caches the resulting defs. Every tenant init after that seeds its cells from that cache and layers the tenant's own readings on top, paying only the user-readings delta.

In practice this turns a full cold compile into a much smaller per-tenant seed. The cache is a module-global, so long-lived processes (Cloudflare Workers warmed up, native daemons) amortize it across every request.

## Concurrency: per-cell isolation

Paper Definition 2 (Cell Isolation) permits concurrent `μ` applications over disjoint cells. The engine implements that by moving per-cell state into `Arc<RwLock<Object>>`, one lock per key in D. Writers follow a two-tier path:

1. **Shared-lock fast path.** Take the outer tenant read lock, snapshot the cells into an `Object::Map`, run `apply()` against that snapshot, then `try_commit_diff`: for each cell whose contents differ, acquire that cell's write lock, CAS against the snapshot value, and commit. Two writers that touch disjoint cells never block each other.
2. **Structural-change escalation.** If the new state adds or removes cells (for example, `compile` introducing new defs), the cells map itself must mutate. The writer drops its read lock, takes the outer write lock, re-snapshots, re-applies, and calls `replace_d`, which rebuilds the cells map while reusing existing locks where keys are unchanged.

A concurrent writer whose CAS check fails (someone else committed a change to a cell it depends on) re-runs from step 1. The scheduler (see `src/scheduler.rs`) orders submissions into three priority lanes — Alethic before Deontic before ReadOnly — so a flood of queries never starves an invariant-critical write.

Snapshots (the MCP `snapshot` / `rollback` verbs) piggy-back on this representation. A capture is one map insert plus an `Arc` ref bump per cell; restoring is `replace_d` against the captured map. Snapshots are per-tenant, so captures in one MCP session do not leak into another.

## WASM lowering (optional)

The evaluator has a second backend. Enable the `wasm-lower` cargo feature and each compiled `Func` can be lowered to a standalone WebAssembly module that executes inside `wasmi`. The lowering covers Backus §11.2.4 combining forms (Id, Compose, Condition, Construction, ApplyToAll, Filter, Insert, While), §11.2.3 primitives (arithmetic, comparisons, logic, sequence builders, distribution, Contains / Lower over string atoms), the Selector family, and host-imported D-access (`Fetch`, `FetchOrPhi`, `Store`, `Def`, `Platform`). `Func::Native` — Rust closures that cannot cross the FFI boundary — is the only intentional gap.

```bash
cargo test --features wasm-lower --lib wasm_lower
```

Object layout in WASM linear memory uses a tag-based encoding: `Atom` = 16 bytes, `Seq` = 8-byte header + 4 bytes per element pointer, `StringAtom` = 8-byte header + length-prefixed bytes. A bump allocator resets between `apply()` calls so each evaluation starts with a fresh heap.

The feature is there for three reasons:

- A buildable artefact for FPGA deployment (see [Generators](07-generators.md) §Verilog — the FPGA bundle plan packages the lowered WASM alongside the synthesized modules).
- Portable dispatch across runtimes where embedding `wasmi` is cheaper than embedding the whole engine.
- Ahead-of-time optimization of Func trees per the FP algebraic laws (paper §Conclusion), since a lowered module is a flat instruction stream amenable to standard compiler passes.

Without the feature flag, the engine evaluates `Func` trees directly in Rust and the WASM path is not included in the binary. Production deployments can run either backend against the same compiled defs.

## What's next

Understanding compile makes the next two files easier. [Generators](07-generators.md) explains what the compiler produces for each runtime target. [Federation](08-federation.md) covers external systems.
