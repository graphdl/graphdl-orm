# 05 · Derivation Rules

A derivation rule says "when these facts are present, this fact is also present." The rule is forward-chained on every `create` until the population reaches a fixed point at which no new derived facts are produced. Derivation is monotonic (rules only add facts, never remove them) and terminating (the population is finite).

## Syntax

A derived fact type has two things: a **mode** that declares how the derivation relates to assertion, and a **rule body** that specifies the condition.

### Mode (marker on the fact type)

Halpin ORM 2 uses three graphical markers on derived fact types. In FORML 2 textual form the marker is a separate token, whitespace-separated, suffixed to the reading in `## Fact Types` and prefixed to the rule in `## Derivation Rules`.

| Marker | Mode | Semantics |
|---|---|---|
| `*` | fully derived | The fact type is always computed from the rule. Asserting directly is a violation. |
| `**` | derived and stored | Same as `*`, but materialized for performance (e.g., a SQL trigger produces the stored column). |
| `+` | semi-derived | The fact type can be computed from the rule, AND may be asserted directly. Used where the rule is a sufficient condition but not the only path. |

The parser translates the marker into a `Fact Type has Derivation Mode` instance fact against the metamodel (see `readings/core.md`). Downstream generators read the mode and decide whether to emit storage, a trigger, or a view.

### Rule body (iff for full, if for partial)

The body uses `iff` for a full derivation (the rule is both necessary AND sufficient) or `if` for a partial derivation (the rule is sufficient; other paths may also populate the fact). Halpin ORM 2 (ORM2.pdf, p. 8): "Iff-rules are used for full derivation, and if-rules for partial derivation."

```forml2
## Fact Types
Customer has Full Name. *

## Derivation Rules
* Customer has Full Name iff Customer has First Name and Customer has Last Name.
```

Partial derivation pairs `+` with `if`:

```forml2
## Fact Types
Person is Grandparent. +

## Derivation Rules
+ Person1 is Grandparent if Person1 is parent of some Person2 and that Person2 is parent of some Person3.
```

The conditional form `If ... then ...` is also recognized for rules whose consequent introduces a new entity:

```forml2
If some User authenticates and that User does not own any Organization then that User owns some Organization.
```

`:=` is retired. It came from pre-ORM 1 BNF-style grammar derivations and is no longer accepted as canonical syntax for new readings. The parser still tolerates it for backward compatibility during migration, but new rules should use the marker + iff/if form.

## Anaphora

The word `that` inside a rule is anaphoric: it refers to an instance introduced earlier by `some`. The rule's join keys are the nouns appearing after `that`.

```forml2
* A has C iff A has some B and that B has some C.
```

Here `B` is the join key: the rule fires when you can find an `A has B` fact and a `B has C` fact sharing the same `B` value.

Multiple anaphors are allowed; the more you use, the tighter the join becomes.

## Kinds of derivation

The compiler classifies rules and emits different Func shapes. All of them lower to the same three compile-functions — `compile_explicit_derivation`, `compile_join_derivation`, `compile_aggregate_derivation` — with no dedicated per-kind implementations (`#287`).

- **Modus ponens** applies when there is no join key and a single antecedent. The compiler lifts every antecedent tuple into the consequent shape.
- **Join** applies when there is one or more `that` anaphor. The compiler computes an equi-join on the shared nouns, then derives the consequent.
- **Subtype inheritance** fires for every (subtype, super-fact-type) pair implied by the schema. Each pair is materialized as a `DerivationRuleDef` whose antecedent source is `InstancesOfNoun(subtype)` and whose consequent cell is the super-fact-type id — one standard 1-antecedent rule per pair, routed through `compile_explicit_derivation`.
- **Transitivity** applies when two binary fact types share a role structure (`A → B` and `B → C`). Each matching pair is materialized as a 2-antecedent Join `DerivationRuleDef` whose consequent cell is a synthesized `_transitive_<ft1>_<ft2>` and whose join key is the shared noun — routed through `compile_join_derivation`.
- **SS auto-fill** fires for every Subset Constraint whose span carries `subset_autofill: true`. Each such constraint materializes a standard 1-antecedent `DerivationRuleDef` copying facts from the antecedent FT to the consequent FT.
- **CWA negation** is generated for nouns under the closed-world assumption. For each (CWA noun, FT) pair where the noun plays a role, the compiler emits derivations into a separate `_cwa_negation:<ft_id>` cell for any instance that doesn't already participate in the FT at that role. Negation facts carry a prefixed `_neg_<noun>` binding key so presence constraints never see them.

The compile-time loops that enumerate these (per-subtype-pair, per-FT-pair, per-SS-constraint, per-CWA-noun-and-FT) are pure schema enumeration, not domain logic — they feed a single pipeline.

## Examples

### Grandparent (semi-derived)

```forml2
## Fact Types
Person is Grandparent. +

## Derivation Rules
+ Person1 is Grandparent if Person1 is parent of some Person2 and that Person2 is parent of some Person3.
```

Classified as Join (shared `Person2`, `Person3`). The `+` mode lets you ALSO assert Grandparent directly when the parent chain is not in the population.

### Access control (semi-derived)

```forml2
## Fact Types
User accesses Domain. +

## Derivation Rules
+ User accesses Domain if User owns Organization and App belongs to that Organization and Domain belongs to that App.
+ User accesses Domain if User administers Organization and App belongs to that Organization and Domain belongs to that App.
+ User accesses Domain if User belongs to Organization and App belongs to that Organization and Domain belongs to that App.
+ User accesses Domain if Domain has Access 'public'.
```

Four partial derivations unioned into the same consequent. Semi-derived because the access check fires whenever any one path holds; no single path is necessary.

### Arity (fully derived, aggregate)

```forml2
## Fact Types
Fact Type has Arity. *

## Derivation Rules
* Fact Type has Arity iff Arity is the count of Role where Fact Type has Role.
```

Aggregate form with `count of ... where ...`. Fully derived: arity is never asserted, only computed.

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

It returns the derivation chain: which rule fired, which antecedent facts the rule consumed, and for each antecedent whether the fact was asserted or itself derived. This is the audit trail, so no authorization decision is a black box.

## SQL triggers for derivations

When a SQL generator is opted in (see [generators](07-generators.md)), the compiler emits `CREATE TRIGGER` statements that materialize derived facts into their own tables. This moves the forward-chain from the engine into the database, which is often faster at scale.

The `**` marker explicitly requests this: a fact type declared `derived and stored` gets a materialized column backed by a trigger. `*` and `+` modes leave the storage decision to the generator's defaults.

The engine detects SQL triggers at compile time and gates its own forward-chain accordingly: only SM-related derivations (the ones needed for status auto-advance) still run in Rust. Everything else is the database's job.

## What's next

You have declared types, constraints, workflows, and inferences. [The compile pipeline](06-compile-pipeline.md) shows what happens when you hand all that to arest.
