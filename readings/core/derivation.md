# Derivation Rule Metamodel (#890)

This file holds the FORML 2 declarative form of the structural
derivation rules that AREST's compiler synthesises during
`compile_to_defs_state`. Each rule is the universal-modus-ponens or
universal-CWA schema from whitepaper §5.2 written against the
metamodel cells (`Subtype`, `FactType`, `Role`, `Noun`).

The compiler's `compile_subtype_inheritance_metamodel` (and the
parallel `compile_derivations` paths for SS auto-fill, transitivity,
CWA negation, SM init) lifts each rule into ONE `CompiledDerivation`
whose Func is the union of the per-binding inner Funcs the rule
quantifies over. The forward chainer fires that Func at evaluation
time exactly as it would fire any user-authored derivation, and the
emitted `<ft_id, reading, bindings>` tuples land in the per-FT cells
the user expects.

## Subtype inheritance (#890 — replaces the per-(sub, sup, ft) Rust loop)

Whitepaper §5.2 universal modus-ponens schema for subtype
membership: every Resource that is an instance of a Subtype is also
an instance of its Supertype, in every Fact Type where the
Supertype plays a Role.

* Fact Type has inherited Resource at Role
    iff some Subtype has subtype Sub and that Subtype has supertype Sup
    and that Fact Type has that Role and that Role is played by Sup
    and that Resource is instance of Sub.

The rule's antecedent quantifies over the four metamodel cells
`Subtype × FactType × Role × <Sub-instances>`; its consequent is the
synthesized `<<Sup-role, Resource>>` binding pushed into every FT
cell where Sup plays the Role. `compile_subtype_inheritance_metamodel`
in `crates/arest/src/compile.rs` performs the lift to a Func:

  Concat . [
    per-(Sub, Sup, FT) inner Func,
    ...
  ]

where each inner Func is `Concat . (apply_to_all per_instance .
instances_of_noun_func(Sub))` — the byte-for-byte same shape
`compile_explicit_derivation` produces for a 1-antecedent
`InstancesOfNoun` rule with `Literal(FT_id)` consequent. Behavioural
equivalence with the pre-#890 per-pair fanout is pinned by
`crates/arest/tests/subtype_metamodel_rule_e2e.rs`.

## SS Subset-Constraint auto-fill (#891 — replaces the per-SS-Constraint Rust loop)

Whitepaper §5.2 universal modus-ponens schema for Subset Constraint
auto-fill: every Fact in the antecedent Fact Type is also a Fact in
the consequent Fact Type, whenever the Subset Constraint's
antecedent span carries the `subset_autofill = true` marker.

* Fact Type has auto-filled Fact
    iff some Subset Constraint has antecedent Fact Type Ant and that
    Subset Constraint has consequent Fact Type Cons and that Subset
    Constraint has autofill 'true' and that Fact is instance of Ant
    and that Fact Type is Cons.

The rule's antecedent quantifies over `Subset-Constraint ×
antecedent-FT-fact` cells; its consequent is the same fact pushed
into the consequent FT cell. `compile_ss_autofill_metamodel` in
`crates/arest/src/compile.rs` performs the lift to a Func:

  Concat . [
    per-SS-Constraint inner Func,
    ...
  ]

where each inner Func is the byte-for-byte same shape
`compile_explicit_derivation` produces for a 1-antecedent
`FactType(antecedent_ft)` rule with `Literal(consequent_ft)`
consequent. Behavioural equivalence with the pre-#891 per-SS-
Constraint fanout is pinned by
`crates/arest/tests/ss_autofill_metamodel_rule_e2e.rs`.

## Other structural rules (deferred — still synthesised in compile.rs)

The following rules currently remain as per-binding loops in
`compile.rs::compile_derivations`. Lifting them to declarative
metamodel rules here is tracked under #287/#311 follow-ups:

* Transitivity of binary FTs — Fact Type has inferred Fact iff some
  Fact uses Resource for the first Role and some other Fact uses
  other Resource for the second Role of a Fact Type sharing the join
  Noun.

* CWA negation — Resource is in complement of FT iff Resource is
  instance of some Noun, Noun plays some Role of FT, no Fact uses
  Resource for that Role.

The original "implicit derivation" framing of subtype inheritance
in `readings/core/core.md` §332 (`Resource is inherited instance of
Noun iff Resource is instance of some subtype of that Noun`) is the
older, looser shape that doesn't address per-FT consequent
materialisation. The rule above is the operational form #890
needs — it spells out the consequent FT and Role explicitly so the
compiler can lift it without guessing.
