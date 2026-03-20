# constraint-eval

A constraint verification and forward inference engine for GraphDL, compiled to WebAssembly.

## Theoretical Foundation

This crate implements an algebra of programs in the sense of Backus's 1977 Turing Lecture, "Can Programming Be Liberated from the von Neumann Style?"

The core insight: **constraints and derivation rules are not data to be interpreted — they are functions to be applied.** The IR (Intermediate Representation) is compiled once into pure functions. Evaluation is function application over whole structures. There are no variables, no mutable state during evaluation, no word-at-a-time dispatch.

### The FP Algebra

Backus described a system where programs are built from primitive functions via combining forms (composition, construction, apply-to-all, condition). This crate implements that system for ORM2 constraint evaluation:

| Backus FP Concept | Implementation |
|---|---|
| **Objects** | `Population` (a set of fact instances), `ResponseContext` (text being evaluated) |
| **Functions** | `Predicate = Fn(&EvalContext) -> Vec<Violation>` — each constraint compiles to a pure function |
| **Combining forms** | Composition of predicates via `flat_map`, construction via `compile_constraint` dispatch, condition via guard predicates on state machine transitions |
| **Definitions** | `CompiledConstraint`, `CompiledDerivation` — named functions bound at compile time |
| **Application** | `(constraint.predicate)(ctx)` — the only operation at evaluation time |

Evaluation of all constraints is a single expression:

```rust
model.constraints.iter().flat_map(|c| (c.predicate)(ctx)).collect()
```

State machine execution is a fold:

```rust
events.fold(initial, |state, event| (machine.transition)(&state, &event, &ctx).unwrap_or(state))
```

These are not implementation choices — they are the algebra. The laws that hold for Backus's FP system (associativity of composition, distributivity of apply-to-all over construction) hold here. Proofs about constraint behavior follow from the algebra, not from tracing execution.

### Why FP, Not von Neumann

A conventional constraint checker would iterate rules, match patterns, branch on types, and accumulate results in mutable state. That approach is:
- Hard to reason about (state changes at every step)
- Hard to parallelize (shared mutable accumulator)
- Hard to prove correct (requires tracing every branch)

The FP approach compiles constraints to closed functions that capture all needed data from the IR at compile time. At evaluation time, each function is independent — it receives an immutable context, produces a result, done. No shared state, no dispatch, no branching. Evaluation is embarrassingly parallel by construction.

## Architecture

```
ConstraintIR (JSON)
    ↓ compile()
CompiledModel
    ├── constraints: [CompiledConstraint]     — Predicate functions
    ├── derivations: [CompiledDerivation]      — Derivation functions
    ├── state_machines: [CompiledStateMachine]  — Transition folds
    └── noun_index: NounIndex                  — Synthesis lookup tables
         ↓ evaluate() / forward_chain() / synthesize()
    [Violation] / [DerivedFact] / SynthesisResult
```

### Compile Phase (once per IR load)

The `compile()` function walks the IR and dispatches on constraint kind **once**. After compilation, the kind is gone — only the predicate function remains.

**Constraint kinds compiled:**
- `UC` — Uniqueness: no duplicate role bindings
- `MC` — Mandatory: every instance must participate
- `RC` — Ring: irreflexivity (no self-reference)
- `XO` — Exclusive-or: exactly one of N clauses holds
- `XC` — Exclusion: at most one of N clauses holds
- `OR` — Inclusive-or: at least one of N clauses holds
- `SS` — Subset: A holds implies B holds
- `EQ` — Equality: A holds iff B holds
- Deontic `forbidden` — text/enum-based violation detection
- Deontic `obligatory` — required presence checking
- Deontic `permitted` — always passes (no constraint)

**Derivation rules compiled:**
- `SubtypeInheritance` — instances of subtypes inherit supertype fact types
- `ModusPonens` — subset constraints produce derived facts (A holds → B holds)
- `Transitivity` — binary fact types sharing a noun produce transitive closure
- `ClosedWorldNegation` — absence of fact implies negation for closed-world nouns

### Evaluate Phase (per request)

Three evaluation modes, all pure function application:

**1. Constraint verification** — `evaluate(&model, &ctx) -> Vec<Violation>`

Apply all compiled predicates, collect violations. Used for deontic constraint checking on agent responses, API input validation, and conformity assessment.

**2. Forward inference** — `forward_chain(&model, &response, &mut population) -> Vec<DerivedFact>`

Apply all derivation rules iteratively until no new facts are produced (fixed point). Maximum 10 iterations to prevent infinite chains. Used for FOL reasoning — given a set of base facts, derive all conclusions.

**3. Synthesis** — `synthesize(&model, &ir, noun_name, depth) -> SynthesisResult`

Collect all knowledge about a noun: participating fact types, applicable constraints, state machines, related nouns, and derived facts. Used to produce compact summaries for agent context injection instead of dumping raw readings.

## World Assumptions

The engine supports dual-mode reasoning via the `WorldAssumption` type on each noun:

**Closed World (default):** If a fact is not in the store and not derivable, it is false. This is the standard database assumption. Applied to government powers, corporate authority, statutory obligations — if the law doesn't grant the power, it doesn't exist.

**Open World:** If a fact is not in the store and not derivable, it is unknown (not false). Applied to individual rights, freedoms, liberties — the absence of an enumerated right does not deny its existence.

This distinction is not an implementation detail. It encodes the 9th and 10th Amendments to the United States Constitution:

- **9th Amendment** (Open World): "The enumeration in the Constitution, of certain rights, shall not be construed to deny or disparage others retained by the people."
- **10th Amendment** (Closed World): "The powers not delegated to the United States by the Constitution... are reserved to the States respectively, or to the people."

The `ClosedWorldNegation` derivation rule only fires for nouns with `WorldAssumption::Closed`. Open-world nouns are left open — the engine reports `Confidence::Incomplete` rather than asserting negation.

## WASM Exports

```rust
// Load and compile constraint IR (call once, or when domain changes)
fn load_ir(ir_json: &str) -> Result<(), JsValue>

// Verify a response against compiled constraints
fn evaluate_response(response_json: &str, population_json: &str) -> String  // → JSON [Violation]

// Collect all knowledge about a noun
fn synthesize_noun(noun_name: &str, depth: usize) -> String  // → JSON SynthesisResult

// Run forward inference on a population
fn forward_chain_population(population_json: &str) -> String  // → JSON [DerivedFact]
```

## CLI

```bash
# Verify text against constraints
graphdl-rules --ir constraints.json --text "response to verify"

# Synthesize knowledge about a noun
graphdl-rules --ir constraints.json --synthesize "AI System" --depth 2

# Forward chain a population
graphdl-rules --ir constraints.json --forward-chain --population facts.json
```

## Tests

27 tests covering:
- All constraint kinds (UC, MC, RC, XO, XC, OR, SS, EQ)
- Deontic modalities (forbidden, obligatory, permitted)
- Subtype inheritance derivation
- Modus ponens from subset constraints
- Transitivity across fact type chains
- CWA vs OWA negation behavior
- Synthesis for known and unknown nouns
- Forward chaining fixed point termination
- Backward-compatible deserialization of old IR format

```bash
cargo test
```
