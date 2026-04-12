# 05 · Derivation Rules

A derivation rule says "when these facts are present, this fact is also present." The rule is forward-chained on every `create` until the population reaches a fixed point — no new derived facts are produced. Derivation is monotonic (rules only add facts, never remove them) and terminating (the population is finite).

## Syntax

Two operators: `:=` for an equivalence by definition, and `iff` for a logical biconditional. Both produce the same compiled form; use whichever reads better.

```forml2
A has Name := A has some B and that B has some Name.
```

or

```forml2
A has Name iff A has some B and that B has some Name.
```

The conditional form `if ... then ...` also works:

```forml2
If some User authenticates and that User does not own any Organization then that User owns some Organization.
```

## Anaphora

The word `that` inside a rule is anaphoric: it refers to an instance introduced earlier by `some`. The rule's join keys are the nouns appearing after `that`.

```forml2
A has C := A has some B and that B has some C.
```

Here `B` is the join key: the rule fires when you can find an `A has B` fact and a `B has C` fact sharing the same `B` value.

Multiple anaphors — the more you use, the tighter the join.

## Kinds of derivation

The compiler classifies rules and emits different Func shapes:

- **Modus ponens** — no join key, one antecedent. Lift every antecedent tuple to the consequent shape.
- **Join** — one or more `that` anaphors. Compute an equi-join on the shared nouns, then derive.
- **Subtype inheritance** — automatic for subtype hierarchies. Inherit all fact types from the supertype.
- **Transitivity** — when two fact types share a role structure (`A → B`, `B → C`), the compiler can derive `A → C`.
- **CWA negation** — for nouns under the closed-world assumption, the compiler generates negation-by-failure: `NOT A has B iff A is instance of Noun and no B exists for A`.

## Examples

### Grandparent

```forml2
Person is a grandparent of Person2 := Person is a parent of some Person3 and that Person3 is a parent of Person2.
```

Classified as Join (shared `Person3`). The derivation scans the `parent of` fact type for all pairs, matches the middle person, and writes a grandparent fact.

### Access control

```forml2
User accesses Domain if User owns Organization and App belongs to that Organization and Domain belongs to that App.
```

Three-way join on `Organization` and `App`. Fires whenever the chain lights up, producing `User accesses Domain` facts that the authorization constraints can check.

### Arity

```forml2
Fact Type has Arity := count of Role where Fact Type has Role.
```

Aggregate form (`count of ... where ...`). Compiles to `length(Filter(...))` over the Role fact type.

### Derivation chain

Rules can reference facts that are themselves derived. The forward chainer runs every derivation once per iteration; any rule whose antecedent now includes a newly-derived fact fires in the next iteration.

The metamodel asserts:

> No Derivation Rule depends on itself.
> If Derivation Rule 1 depends on Derivation Rule 2, then Derivation Rule 2 does not depend on Derivation Rule 1.

Cyclic derivations are rejected at compile time.

## Fixed point

On every `create`, the derivation engine runs all relevant derivation rules over the current population, adding new facts. It repeats until no rule produces a new fact (the least fixed point). The paper guarantees this exists and is unique (Theorem 3: Completeness).

In practice the compiler gates by noun: only rules whose antecedent or consequent fact types involve the noun being created (plus their transitive dependents) run. This is the `derivation_index:{noun}` cell compiled per-noun. See [compile pipeline](06-compile-pipeline.md) for details.

## Explaining a derived fact

Every derived fact is traced. Call `explain` on a fact to see the chain of rules that produced it:

```bash
# via MCP
explain { fact_type: "User_accesses_Domain", bindings: { User: "alice", Domain: "core" } }
```

Returns the derivation chain: which rule fired, which antecedent facts it consumed, and for each antecedent whether it was asserted or itself derived. This is the audit trail — no black-box authorization decisions.

## SQL triggers for derivations

When a SQL generator is opted in (see [generators](07-generators.md)), the compiler emits `CREATE TRIGGER` statements that materialize derived facts into their own tables. This moves the forward-chain from the engine into the database, which is often faster at scale.

The engine detects SQL triggers at compile time and gates its own forward-chain accordingly: only SM-related derivations (the ones needed for status auto-advance) still run in Rust. Everything else is the database's job.

## What's next

You have declared types, constraints, workflows, and inferences. [The compile pipeline](06-compile-pipeline.md) shows what happens when you hand all that to arest.
