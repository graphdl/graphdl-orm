# arest

A constraint verification and forward inference engine for AREST, compiled to WebAssembly.

## Theoretical Foundation

This crate implements an algebra of programs in the sense of Backus's 1977 Turing Lecture, "Can Programming Be Liberated from the von Neumann Style?"

The core insight is that **constraints and derivation rules are not data to be interpreted; they are functions to be applied.** The IR (Intermediate Representation) is compiled once into pure functions. Evaluation is function application over whole structures. There are no variables, no mutable state during evaluation, and no word-at-a-time dispatch.

### The FP Algebra

Backus described a system in which programs are built from primitive functions via combining forms (composition, construction, apply-to-all, condition). This crate implements that system for ORM2 constraint evaluation:

| Backus FP Concept | Implementation |
|---|---|
| **Objects** | `Population` (a set of fact instances) and `ResponseContext` (text being evaluated). |
| **Functions** | `Predicate = Fn(&EvalContext) -> Vec<Violation>`. Each constraint compiles to a pure function. |
| **Combining forms** | Predicates compose via `flat_map`, construction goes through `compile_constraint` dispatch, and conditions go through guard predicates on state machine transitions. |
| **Definitions** | `CompiledConstraint` and `CompiledDerivation` are named functions bound at compile time. |
| **Application** | `(constraint.predicate)(ctx)` is the only operation at evaluation time. |

Evaluation of all constraints is a single expression:

```rust
model.constraints.iter().flat_map(|c| (c.predicate)(ctx)).collect()
```

State machine execution is a fold:

```rust
events.fold(initial, |state, event| (machine.transition)(&state, &event, &ctx).unwrap_or(state))
```

These are not implementation choices; they are the algebra. The laws that hold for Backus's FP system (associativity of composition, distributivity of apply-to-all over construction) hold here too. Proofs about constraint behavior follow from the algebra, not from tracing execution.

### Why FP, Not von Neumann

A conventional constraint checker would iterate rules, match patterns, branch on types, and accumulate results in mutable state. That approach is hard for several reasons:

- It is hard to reason about, since state changes at every step.
- It is hard to parallelize, since the accumulator is shared and mutable.
- It is hard to prove correct, since proof requires tracing every branch.

The FP approach compiles constraints to closed functions that capture all needed data from the IR at compile time. At evaluation time, each function is independent. It receives an immutable context and produces a result. There is no shared state, no dispatch, and no branching. Evaluation is embarrassingly parallel by construction.

## Architecture

```mermaid
flowchart TD
    IR["ConstraintIR (JSON)"] -->|compile()| CM[CompiledModel]
    CM --> CC["constraints: CompiledConstraint<br/>(predicate functions)"]
    CM --> CD["derivations: CompiledDerivation<br/>(derivation functions)"]
    CM --> SM["state_machines: CompiledStateMachine<br/>(transition folds)"]
    CM --> NI["noun_index: NounIndex<br/>(synthesis lookup tables)"]
    CC -->|evaluate()| V["[Violation]"]
    CD -->|forward_chain()| DF["[DerivedFact]"]
    NI -->|synthesize()| SR[SynthesisResult]
```

### Compile Phase (once per IR load)

The `compile()` function walks the IR and dispatches on the constraint kind **once**. After compilation, the kind is gone, and only the predicate function remains.

The **constraint kinds** that the compiler handles are:

- `UC` (Uniqueness): no duplicate role bindings.
- `MC` (Mandatory): every instance must participate.
- `RC` (Ring): irreflexivity, with no self-reference.
- `XO` (Exclusive-or): exactly one of N clauses holds.
- `XC` (Exclusion): at most one of N clauses holds.
- `OR` (Inclusive-or): at least one of N clauses holds.
- `SS` (Subset): A holds implies B holds.
- `EQ` (Equality): A holds iff B holds.
- Deontic `forbidden`: text and enum-based violation detection.
- Deontic `obligatory`: required-presence checking.
- Deontic `permitted`: always passes (no constraint).

The **derivation rules** that the compiler handles are:

- `SubtypeInheritance`: instances of subtypes inherit supertype fact types.
- `ModusPonens`: subset constraints produce derived facts (A holds, therefore B holds).
- `Transitivity`: binary fact types sharing a noun produce the transitive closure.
- `ClosedWorldNegation`: the absence of a fact implies negation for closed-world nouns.

### Evaluate Phase (per request)

Three evaluation modes operate by pure function application.

**1. Constraint verification.** `evaluate(&model, &ctx) -> Vec<Violation>` applies all compiled predicates and collects violations. This is used for deontic constraint checking on agent responses, API input validation, and conformity assessment.

**2. Forward inference.** `forward_chain(&model, &response, &mut population) -> Vec<DerivedFact>` applies all derivation rules iteratively until no new facts are produced (the fixed point). The cap is ten iterations to prevent infinite chains. This is used for FOL reasoning: given a set of base facts, the engine derives all conclusions.

**3. Synthesis.** `synthesize(&model, &ir, noun_name, depth) -> SynthesisResult` collects all knowledge about a noun, including participating fact types, applicable constraints, state machines, related nouns, and derived facts. This produces compact summaries for agent context injection without dumping raw readings.

## World Assumptions

The engine supports dual-mode reasoning via the `WorldAssumption` type on each noun.

**Closed World (default):** if a fact is not in the store and not derivable, it is false. This is the standard database assumption. It applies to permissions and corporate authority: if the permission is not granted, it does not exist.

**Open World:** if a fact is not in the store and not derivable, it is unknown rather than false. It applies to capabilities and unenumerated abilities, since the absence of an explicit capability does not deny its existence.

This distinction is not an implementation detail. It encodes the 9th and 10th Amendments to the United States Constitution:

- **9th Amendment** (Open World): "The enumeration in the Constitution, of certain rights, shall not be construed to deny or disparage others retained by the people."
- **10th Amendment** (Closed World): "The powers not delegated to the United States by the Constitution... are reserved to the States respectively, or to the people."

The `ClosedWorldNegation` derivation rule fires only for nouns with `WorldAssumption::Closed`. Open-world nouns are left open, so the engine reports `Confidence::Incomplete` rather than asserting negation.

## WASM Exports

```rust
// Load and compile constraint IR (call once, or when the domain changes)
fn load_ir(ir_json: &str) -> Result<(), JsValue>

// Verify a response against compiled constraints
fn evaluate_response(response_json: &str, population_json: &str) -> String  // returns JSON [Violation]

// Collect all knowledge about a noun
fn synthesize_noun(noun_name: &str, depth: usize) -> String  // returns JSON SynthesisResult

// Run forward inference on a population
fn forward_chain_population(population_json: &str) -> String  // returns JSON [DerivedFact]
```

## CLI

```bash
# Verify text against constraints
fol --ir constraints.json --text "response to verify"

# Synthesize knowledge about a noun
fol --ir constraints.json --synthesize "AI System" --depth 2

# Forward chain a population
fol --ir constraints.json --forward-chain --population facts.json
```

## Tests

There are 27 tests, covering:

- All constraint kinds (UC, MC, RC, XO, XC, OR, SS, EQ).
- Deontic modalities (forbidden, obligatory, permitted).
- Subtype inheritance derivation.
- Modus ponens from subset constraints.
- Transitivity across fact-type chains.
- CWA versus OWA negation behavior.
- Synthesis for known and unknown nouns.
- Forward chaining at the fixed-point termination.
- Backward-compatible deserialization of the old IR format.

```bash
cargo test
```
