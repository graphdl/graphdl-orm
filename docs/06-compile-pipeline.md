# 06 · The Compile Pipeline

This doc walks through what happens between the moment you hand the engine a directory of readings and the moment it is ready to answer queries. You do not need to understand every step to use graphdl-orm, but understanding the pipeline helps when debugging, optimizing, or extending the system.

## One-line summary

```
readings text → parse → Domain → domain_to_state → P (Map) → compile_to_defs_state → defs (Vec) → defs_to_state → D (Map)
```

`P` is the population (facts). `D` is the state — `P` plus all compiled defs in a single keyed Map. Every MCP call operates over `D`.

## Step 1: Parse

`parse_markdown` reads each reading and classifies it into one of three families (Theorem 1: Grammar Unambiguity):

- Quantified constraints (`Each X has some Y`, `For each X, at most one Y ...`)
- Conditional constraints (`If ... then ...`, `... iff ...`)
- Multi-clause constraints (`exactly one of the following holds ...`)

Nouns are matched longest-first so that multi-word names like `State Machine Definition` are recognized before `State` alone. Unknown nouns are auto-created in permissive mode, or rejected in `--strict` mode.

The parser produces a `Domain` struct containing:

- `nouns` — entity and value type declarations
- `fact_types` — all declared fact types with their roles
- `constraints` — every constraint with its spans and modality
- `state_machines` — SM definitions and their transitions
- `derivation_rules` — rules with resolved antecedents and consequents
- `general_instance_facts` — instance facts and subject/object/field bindings

Parse is currently the slowest step at scale (~58 µs per fact type or instance fact). It only runs once per compile, and per-command `create` does not parse — it reads already-compiled defs.

## Step 2: `domain_to_state`

The `Domain` struct is turned into a single `Object::Map` where each cell holds a sequence of facts. One cell per category (Noun, GraphSchema, Constraint, DerivationRule, ...) plus one cell per declared fact type ID (for instance facts).

This step is O(n) in the number of facts. Cells are built mutably and wrapped in a `Map` once at the end — cheaper than the equivalent fold over `cell_push` which would be O(n²).

## Step 3: `compile_to_defs_state`

This is the big one. It reconstructs a Domain from the state (round-trip; the engine operates over the Object representation, not the struct), then produces a flat list of `(name, Func)` pairs:

- **Constraints** — each gets `constraint:{id}` plus `validate:{fact_type_id}` per-FT indexed validators.
- **Machines** — `machine:{noun}`, `machine:{noun}:initial`, `transitions:{noun}`.
- **Derivations** — `derivation:{id}` for each user-defined and synthetic (CWA negation, subtype inheritance, transitivity) derivation.
- **Derivation index** — `derivation_index:{noun}` cells so `create` can gate which rules run.
- **Shard map** — `shard:{fact_type_id}` mapping each fact type to its owning cell (RMAP partition).
- **Schemas** — `schema:{fact_type_id}` Construction funcs (tuple constructors).
- **Resolve** — `resolve:{noun}` condition chain mapping field name to fact type.
- **Query** — `query:{fact_type_id}` returning role metadata.
- **Populate** — `populate:{noun}` for federated nouns.
- **Generators** — `sql:sqlite:{table}`, `xsd:{noun}`, `ilayer:{noun}`, `test:{id}` — any generator whose opt-in appears as an instance fact.

The result is a vector of named Funcs. Debug output during compile shows timing per phase.

## Step 4: `defs_to_state`

Merge the compiled defs with the existing state cells into a single `Object::Map`. Every def becomes a key; every cell becomes a key; lookup is O(1). This is the `D` the rest of the runtime consumes.

## RMAP: relational mapping

[Halpin Ch. 10] gives the procedure. The engine's RMAP:

1. Binarize exclusive unaries (XO unary fact types become a status column on the entity).
2. Absorb subtypes — partitioned strategy if the subtype has its own fact types, single-table otherwise.
3. Classify fact types by UC arity:
   - **Compound UC** (spanning ≥ 2 roles) → its own M:N table.
   - **Single-role UC** → functional; absorb into the entity's table with the other role as a column.
   - **No UC** → junction table.
4. 1:1 absorption — when both roles have single-role UCs, absorb into the mandatory side; if neither is mandatory, absorb into the entity-over-value or the larger table.
5. Compound reference schemes — composite primary key on the components.
6. Constraint mapping — UC → UNIQUE, MC → NOT NULL, VC → CHECK, SS → foreign key.

The result is a list of `TableDef` structs with columns, primary keys, uniqueness constraints, and check constraints. SQL and FPGA generators both consume this same structure.

## Hot paths

On every `create`:

1. `resolve` — identity from the ref scheme. Runtime functions (federation, external calls) execute here.
2. `derive` — forward-chain relevant derivation rules to LFP.
3. `validate` — apply every constraint as a restriction over P. Gated by fact type if possible.
4. SM fold — consume the events generated by resolve/derive, advance status.
5. `emit` — construct the representation `⟨P', V, links⟩`.

Constraints are indexed: `validate:{fact_type_id}` runs only the constraints that span that FT. The full `validate` is only needed when you do not know which FT changed.

Derivations are indexed by noun: `derivation_index:{Order}` lists the rule IDs relevant to Orders. The engine only forward-chains those.

Fetches against `D` are O(1) (backed by `HashMap`). At scale (100 fact types, 2,000+ defs), one fetch is ~0.3 µs.

## Incremental compile

`compile` can be called on a running system to add new readings (Corollary 5: Closure Under Self-Modification). The new definitions are merged into `DEFS` via `↓DEFS`; subsequent `SYSTEM` applications see them. This is how `propose` eventually lands: a Domain Change transitioning to Applied invokes `compile` on the proposed readings.

## What's next

Understanding compile makes the next two files easier. [Generators](07-generators.md) explains what the compiler produces for each runtime target. [Federation](08-federation.md) covers external systems.
