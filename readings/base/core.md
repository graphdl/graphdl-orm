# AREST Core Metamodel

## Entity Types

Function(.id) is an entity type.
Noun is a subtype of Function.
  Event Type is a subtype of Resource.
  Fact Type is a subtype of Event Type.
  {Event Type, Status, State Machine Definition, Constraint, Derivation Rule} are mutually exclusive subtypes of Resource.

Reading(.id) is an entity type.

Role(.id) is an entity type.

Verb is a subtype of Function.
  HTTP Method is a subtype of Verb.

Constraint(.id) is an entity type.
  Constraint is a subtype of Resource.
  Set Comparison Constraint is a subtype of Constraint.
  Frequency Constraint is a subtype of Constraint.
  {Set Comparison Constraint, Frequency Constraint} are mutually exclusive subtypes of Constraint.

Constraint Type(.code) is an entity type.

Derivation Rule(.id) is an entity type.
  Derivation Rule is a subtype of Resource.

Modality Type is a value type.
  The possible values of Modality Type are 'Alethic', 'Deontic'.

World Assumption is a value type.
  The possible values of World Assumption are 'closed', 'open'.

Language(.code) is an entity type.

schema:Thing(.Name) is an entity type.

External System(.Name) is an entity type.

## Value Types

URL is a value type.
Secret Reference is a value type.
Reference Scheme is a value type.

id is a value type.
code is a value type.
Arity is a value type.
Position is a value type.
Min Occurrence is a value type.
Max Occurrence is a value type.
Name is a value type.
Plural is a value type.
Object Type is a value type.
  The possible values of Object Type are 'entity', 'value'.
<!-- `Format` was a value type here (legacy widget Format: 'text', 'date',
     'boolean'). It is PROMOTED to a first-class, extensible entity type
     `Format(.Name)` in the NORMA Value Domain section below (alongside
     `Conceptual Data Type`), so new presentation Formats are added by
     declaring instances rather than editing a closed enumeration. The
     `Noun has Format` binary at :109 is unchanged in surface syntax but is
     now an entity-valued refinement link (Noun -> Format entity). No live
     reading or app ever populated `Noun has Format`, so the promotion is
     data-safe. See "## NORMA Value Domain" -> Format. -->
Enum Values is a value type.
Minimum is a value type.
Maximum is a value type.
Exclusive Minimum is a value type.
Exclusive Maximum is a value type.
Multiple Of is a value type.
Min Length is a value type.
Max Length is a value type.
Pattern is a value type.
Description is a value type.
Text is a value type.
URI is a value type.
Prefix is a value type.
Header is a value type.
Timestamp is a value type.
Argument Length is a value type.
Order is a value type.
Data is a value type.
Result is a value type.
Title is a value type.

Permission is a value type.
  The possible values of Permission are 'create', 'read', 'update', 'delete', 'list', 'versioned', 'login', 'rateLimit'.

Role Relationship is a value type.
  The possible values of Role Relationship are 'many-to-one', 'one-to-many', 'many-to-many', 'one-to-one'.


Scope is a value type.
  The possible values of Scope are 'organization', 'public'.

Derivation Mode is a value type.
  The possible values of Derivation Mode are 'fully-derived', 'derived-and-stored', 'semi-derived'.

Constraint Type Label is a value type.

Constraint Type Family is a value type.
  The possible values of Constraint Type Family are 'ring', 'uniqueness', 'mandatory', 'frequency', 'value', 'set-comparison', 'subset', 'equality', 'deontic'.

Constraint Match Keyword is a value type.

## Fact Types

### Noun
Noun has Object Type.
  Each Noun has exactly one Object Type.
Noun has Plural.
  Each Noun has at most one Plural.
Noun has value-type- Name.
  Each value-type- Name belongs to at most one Noun.
Noun has Format.
  Each Noun has at most one Format.
Noun has Enum Values.
  Each Noun has at most one Enum Values.
Noun has Minimum.
  Each Noun has at most one Minimum.
Noun has Maximum.
  Each Noun has at most one Maximum.
Noun has Pattern.
  Each Noun has at most one Pattern.
Noun has Description.
  Each Noun has at most one Description.
Noun has Exclusive Minimum.
  Each Noun has at most one Exclusive Minimum.
Noun has Exclusive Maximum.
  Each Noun has at most one Exclusive Maximum.
Noun has Multiple Of.
  Each Noun has at most one Multiple Of.
Noun has Min Length.
  Each Noun has at most one Min Length.
Noun has Max Length.
  Each Noun has at most one Max Length.
Noun has Permission.
Noun has Reference Scheme.
  Each Noun has at most one Reference Scheme.
  <!-- task-961 Phase A: a VALUE-typed presence projection of the absorbed
       `referenceScheme` field on the Noun cell. `Reference Scheme` is a
       value type, so this functional binary is RMAP-absorbed into the Noun
       cell (no own data cell) and `rmap::reconstitute_absorbed_ft` projects
       it back out as `<<Noun, X>, <Reference Scheme, "id,…">>` for exactly
       those Nouns whose `referenceScheme` key is present (an entity with a
       declared `(.col)` reference scheme). lower_camel("Reference Scheme")
       == "referenceScheme", so reconstitution locates the stored value.
       This is the materializable 2nd conjunct of `Noun is instantiable`
       below — it replaces the entity-valued `Noun has Reference Scheme
       Noun`, which never populated for real entities (their identity lives
       in the absorbed field, not an entity-valued fact). -->
Noun is subtype of Noun.
Noun is described to AI by prompt Text.
Noun has World Assumption.
  Each Noun has exactly one World Assumption.
Noun is independent.
Noun is of schema:Thing.
  Each Noun is of at most one schema:Thing.
  It is possible that more than one Noun is of the same schema:Thing.
Noun plays Role.
  It is obligatory that each Noun plays some Role.
  For each Role, exactly one Noun plays that Role.
  It is possible that some Noun plays more than one Role.
Noun is instantiable. **
  <!-- task-961 lift (shipped bdde710d/a9a78c74): a Noun is instantiable iff
       it is an entity type (objectType='entity') AND it has a reference
       scheme (identity). The derivation under ## Derivation Rules below
       carries the logic; the Rust create/update gate at
       command.rs::noun_runtime_defined reads this stored cell first.

       task-961 Phase A (derivation rework): the 2nd conjunct now reads the
       VALUE-typed presence projection `Noun has Reference Scheme` (above),
       which reconstitutes from the absorbed `referenceScheme` field — so the
       derivation MATERIALIZES the real entity types (Task, Source File, App,
       Domain, …). Previously the 2nd conjunct pointed at the entity-valued
       `Noun has Reference Scheme Noun`, which is never populated for real
       entities, so this cell stayed empty for them and the procedural
       fallback alone carried the gate. The `**` marker stores the consequent.

       task-961 Phase B: the alethic instantiability constraint below makes
       the rejection of a non-instantiable-noun create/update DECLARATIVE.
       `command.rs::noun_runtime_defined` treats the cell as the AUTHORITATIVE
       source whenever it is NON-EMPTY. A create of a noun absent from a
       populated `Noun_is_instantiable` is rejected (D' = D, per AREST.tex
       eq:create §157). The procedural Noun-cell scan was retained as a
       fallback ONLY for states where the cell was still empty.

       task-961 Phase C (this codebase): `compile_to_defs_state` now ALWAYS
       emits `_Noun_is_instantiable_compiled` (same predicate: objectType='entity'
       AND non-empty referenceScheme, evaluated at compile time against the Noun
       cell). `noun_instantiable_per_cell` checks BOTH `Noun_is_instantiable`
       (forward-chain-produced) AND `_Noun_is_instantiable_compiled` (compile-time
       constant, with FFP `[', Seq]` wrapper unwrapped), providing a fast-path
       declarative admit for any noun known at compile time.  The procedural
       fallback `noun_runtime_defined_procedural` is RETAINED for:
         (a) states built without `compile_to_defs_state` (phi-state test
             fixtures like `apply_command_phi_state()`), and
         (b) nouns added to `state` dynamically after the last compile.
       Full procedural removal requires guaranteeing every `apply` path passes
       through `compile_to_defs_state` — a follow-up child task.
       Oracle-equivalence pinned by
       `compile_noun_is_instantiable_compile_time_cell_matches_procedural_predicate`
       in compile.rs. -->

It is impossible that a Resource is an instance of a Noun that is not instantiable.
  <!-- task-961 Phase B/C — the declarative instantiability constraint. ALETHIC
       (AREST.tex §328 "It is impossible that …"): instantiating an entity of
       a noun that is not in either the derived `Noun_is_instantiable` cell OR
       the compile-time `_Noun_is_instantiable_compiled` cell is a structural
       impossibility and rejects (D' = D). The check is a set-membership test
       whose predicate logic lives in the `Noun is instantiable` derivation
       and the compile-time materialisation in `compile_to_defs_state`.
       Evaluated by `command.rs::noun_runtime_defined` as the create/update
       run-time gate (with procedural fallback for uncomplied states). -->

### Reading
Reading has Text.
  Each Reading has exactly one Text.
  It is possible that more than one Reading has the same Text.
Reading is used by Verb.
  Each Reading is used by exactly one Verb.
  It is possible that some Verb is used by more than one Reading.
Reading is localized for Language.
  Each Reading is localized for at most one Language.
  It is possible that more than one Reading is localized for the same Language.
Reading is primary.
Role is used in Reading.
  Each Role is used in some Reading.
  For each Reading, some Role is used in that Reading.

### Fact Type (subtype of Noun)
Fact Type has Title.
  Each Fact Type has at most one Title.
Fact Type has Reading.
  Each Fact Type has some Reading.
  For each Reading, exactly one Fact Type has that Reading.
  It is possible that some Fact Type has more than one Reading.
Fact Type has Role.
  Each Fact Type has some Role.
  For each Role, exactly one Fact Type has that Role.
  It is possible that some Fact Type has more than one Role.
Fact Type has Arity. *
  Each Fact Type has exactly one Arity.
Fact Type has Order.
  Each Fact Type has at most one Order.
Fact Type has Role Relationship.
  Each Fact Type has at most one Role Relationship.
Fact Type has Derivation Mode.
  Each Fact Type has at most one Derivation Mode.

### Role
Constraint spans Role.
  Each Constraint spans some Role.
  This association with Constraint, Role provides the preferred identification scheme for Constraint Span.
Role has Position for Reading.
  For each Role and Reading that Role has that Reading at most one Position.

### Verb
Verb has Name.
  Each Verb has exactly one Name.
  It is possible that more than one Verb has the same Name.
Fact Type is activated by Verb.
  In each population of Fact Type is activated by Verb, each Fact Type, Verb combination occurs at most once.
  This association with Fact Type, Verb provides the preferred identification scheme for API.
Fact is referenced by Verb.
  It is possible that some Verb references more than one Fact.
  It is possible that more than one Verb references the same Fact.
<!-- Verb is performed during Transition (Mealy semantics). -->
  For each Transition, at most one Verb is performed during that Transition.
  It is possible that some Verb is performed during more than one Transition.
<!-- Verb is performed in Status (Moore semantics). -->
  For each Status, at most one Verb is performed in that Status.
  It is possible that some Verb is performed in more than one Status.

### Function
Function has Name.
  Each Function has at most one Name.
Function has callback URI.
  Each Function has at most one callback URI.
Function has Header.
  Each Function has each Header at most once.
Function has Scope.
  Each Function has at most one Scope.
Function belongs to Domain.
  Each Function belongs to at most one Domain.
It is obligatory that each Function belongs to some Domain.

### Constraint
Constraint has modality of Modality Type.
Constraint has Text.
  Each Constraint has at most one Text.
Constraint is semantic.
Constraint has Constraint Match Keyword.
  It is possible that some Constraint has more than one Constraint Match Keyword.

### Constraint Type (merged #13: NORMA ConstraintType — one classifier carrying code, Name, Label, Family, and Violation Template)
Constraint Type has Name.
  Each Constraint Type has at most one Name.
Constraint Type has Constraint Type Label.
  Each Constraint Type has exactly one Constraint Type Label.
Constraint Type has Constraint Type Family.
  Each Constraint Type has exactly one Constraint Type Family.

### Set Comparison Constraint (subtype of Constraint)
Set Comparison Constraint has Argument Length.

### Frequency Constraint (subtype of Constraint)
Frequency Constraint has Min Occurrence.
  Each Frequency Constraint has exactly one Min Occurrence.
Frequency Constraint has Max Occurrence.
  Each Frequency Constraint has at most one Max Occurrence.

### Constraint Span (objectification of "Constraint spans Role")
Constraint Span autofills from superset.

### Stream
Stream has Name.
  Each Stream has exactly one Name.
  It is possible that more than one Stream has the same Name.

### API (objectification of "Fact Type is activated by Verb")
API accepts Noun as parameter.
  Each API, Noun combination occurs at most once in the population of API accepts Noun as parameter.

## Constraints

Each Constraint has modality of exactly one Modality Type.
It is possible that more than one Constraint has modality of the same Modality Type.

## Disjunctive Mandatory Constraints

For each Status, some Transition is from that Status or some Transition is to that Status.


## Subset Constraints

If some Role is used in some Reading where some Fact Type has that Reading then that Fact Type has that Role.
If some Fact uses some Resource for some Role then that Fact is of some Fact Type that has that Role.
If some Fact uses some Resource for some Role then that Resource is instance of some Noun that plays that Role.
If some Fact Type defines some Fact then some Resource that is that Fact is instance of some Noun that is that Fact Type.
If some Verb references some Fact that is of some Fact Type then that Verb uses some Reading where that Fact Type has that Reading.
If some Guard Run is for some Guard and that Guard Run references some Fact then that Guard references some Fact Type that defines that Fact.
If some State Machine is currently in some Status then that Status is defined in some State Machine Definition where that State Machine is instance of that State Machine Definition.
If some API accepts some Noun as parameter and some other Noun is subtype of that Noun then that API accepts that subtype Noun as parameter.
If some Noun has some Format then that Noun has some Conceptual Data Type.
If some Noun has some Format then that Format is built on some Conceptual Data Type.

## Ring Constraints

No Noun is subtype of itself.
If Noun1 is subtype of Noun2, then Noun2 is not subtype of Noun1.
If Noun1 is subtype of Noun2 and Noun2 is subtype of Noun3, then Noun1 is subtype of Noun3.

No Derivation Rule depends on itself.
If Derivation Rule 1 depends on Derivation Rule 2 and Derivation Rule 2 depends on Derivation Rule 3, then Derivation Rule 1 does not depend on Derivation Rule 3.

### External System
External System has URL.
  Each External System has exactly one URL.
External System has Header.
  Each External System has at most one Header.
External System has Prefix.
  Each External System has at most one Prefix.
External System has Kind.
  Each External System has at most one Kind.
Noun is backed by External System.
  Each Noun is backed by at most one External System.
Function is backed by External System.
  Each Function is backed by at most one External System.

Noun has URI.
  Each Noun has at most one URI.

### Domain Connection
Domain connects to External System with Secret Reference.
  Each Domain has at most one Secret Reference per External System.

### Derivation Rule

Derivation Rule(.id) is an entity type.
Derivation Rule has Text.
  Each Derivation Rule has exactly one Text.
Derivation Rule has antecedent Fact Type.
Derivation Rule produces Fact Type.
  Each Derivation Rule produces exactly one Fact Type.
Derivation Rule depends on Derivation Rule. *

## Derivation Rules

* Fact Type has Arity iff Arity is the count of Role where Fact Type has Role.

* Derivation Rule depends on Derivation Rule iff Derivation Rule has antecedent Fact Type and some other Derivation Rule produces that Fact Type.

* Noun is instantiable iff Noun has Object Type 'entity' and Noun has some Reference Scheme.

Constraint is semantic iff Constraint has modality of Modality Type 'Deontic' and Constraint spans some Role and that Role is played by some Noun and no Resource is instance of that Noun.

## Implicit Derivation Rules (#316 / #287c)

<!--
The four derivations below are currently materialised by the
compiler's `compile_derivations` synthesis pass (per-subtype, per-SS,
per-noun-FT, per-binary-pair fan-out). Expressing them as rules in the
metamodel closes the loop: the parser will drive them straight from
these readings once #317 lands anaphora + subscript + metamodel-cell
push; until then the Rust synthesis continues to cover them.
-->

### Subtype inheritance

Every fact that binds a subtype also binds the supertype: if `Noun1`
is a subtype of `Noun2` and a Fact uses a Resource whose Noun is
`Noun1` for some Role, then that same Resource is also an instance of
`Noun2`. In ORM this IS `Resource is instance of Noun` — subtyping is
population inclusion (Halpin, "Subtyping Revisited": all instances of a
type are instances of its supertype), so instance-of is transitive and
the runtime mirror is deliberately over-broad to carry it. Inheritance
proper is PROPERTY reuse (a subtype plays the supertype's roles because
it IS a supertype instance), not a distinct membership relation.

<!-- RETIRED 2026-07-09 (challenged + NORMA-verified): `Resource is
     inherited instance of Noun` was a non-canonical relation — ORM has
     no separate "inherited membership", and it only existed to prop up
     the (also non-canonical, now relaxed) `instance of exactly one
     Noun`. It had ZERO readers in base or apps (grep: only its own
     declaration), so retiring it removes dead derived data.
* Resource is inherited instance of Noun iff Resource is instance of some subtype of that Noun. -->


### Subset Constraint auto-fill (SS)

<!--
Each declared Subset Constraint whose `autofill` span marker is true
copies every antecedent fact into the consequent Fact Type. One
DerivationRule per SS constraint, each routed through
`compile_explicit_derivation` as a single-antecedent rule.
-->

* Fact is in consequent Fact Type iff some Subset Constraint has autofill 'true' and Subset Constraint spans antecedent Fact Type and Fact is instance of that antecedent Fact Type.

### Transitivity of binary Fact Types

<!--
For each pair of binary Fact Types `(A R B, B R C)` where the second
Role of the first FT and the first Role of the second FT share a Noun,
emit inferred `A R C` facts. Compile-time enumerates FT pairs; runtime
derives one fact per join.
-->

* Fact Type has inferred Fact iff some Fact uses Resource for the first Role of that Fact Type and some other Fact uses other Resource for the second Role of a Fact Type sharing the join Noun.

## Check-Readings Deontic Obligations (#288)

<!--
Layers 2 and 3 of the readings checker (`crates/arest/src/check.rs`)
enforce ring-constraint validity and completeness as Rust control
flow today. Expressing them as deontic constraints here lets #317's
metamodel-FT push eventually drive them through Theorem 4's
violation path — the Rust layers retire, and authors see the same
diagnostics via the standard violation surface.
-->

### Layer 2: ring validity — same-noun spans

<!-- A ring constraint (`IR`, `AS`, `AT`, `SY`, `IT`, `TR`, `AC`, `RF`)
must span roles whose Nouns are identical. A ring across mixed
nouns is nonsensical — `No Customer is-subtype-of Address` has
nothing to forbid. check.rs emits an Error-level diagnostic
today; the deontic form is the same invariant spelled
declaratively. -->

It is obligatory that each Ring Constraint spans two Roles and both Roles are played by the same Noun.

### Layer 3: ring completeness — declare the ring on a same-noun binary

<!-- A binary Fact Type whose two Roles share the same Noun almost
always wants an explicit ring constraint — without one, nothing
prevents the self-reference cycle the schema is implicitly
modelling. check.rs emits a Hint-level diagnostic that points
authors at the missing `is acyclic.` / `is irreflexive.`
annotation. -->

It is obligatory that each binary Fact Type whose Roles are played by the same Noun has some Ring Constraint spanning it.

## NORMA Structural Decomposition (#279)

<!--
The concepts below mirror NORMA's `ORMCoreMetaModel.orm`
decomposition of derivation rule bodies. They are the FORML 2
surface that the meta-circular parser (#280) populates by
decomposing each user-authored rule into a `Join Path` +
`Role Sequence` + `Role Projection`, rather than classifying the
rule text with Rust heuristics.
-->

Paper §4 Table 1 correspondence:
  Join Path       ↔ Composition (COMP)
  Role Sequence   ↔ Construction (CONS)
  Role Projection ↔ Selector
  Join Type       ↔ Condition (COND)

### Entity types

Join Path(.id) is an entity type.
Join(.id) is an entity type.
Role Sequence(.id) is an entity type.
Role Projection(.id) is an entity type.
Join Type(.Name) is an entity type.

### Value types

Clusivity is a value type.
  The possible values of Clusivity are 'inclusive', 'exclusive'.

Derivation Storage Type is a value type.
  The possible values of Derivation Storage Type are 'stored', 'derived', 'derived-and-stored'.

### Fact types

Derivation Rule has Join Path.
  Each Derivation Rule has at most one Join Path.

Join Path has Join.
  Each Join Path has some Join.
  For each Join, exactly one Join Path has that Join.

Join uses Fact Type.
  Each Join uses exactly one Fact Type.

Join has Join Type.
  Each Join has exactly one Join Type.

Join has Role Sequence.
  Each Join has some Role Sequence.

Role Sequence has Role at Position.
  For each Role Sequence and Position, at most one Role is at that Position in that Role Sequence.

Role Projection is from Role Sequence.
  Each Role Projection is from exactly one Role Sequence.

Role Projection produces Role.
  Each Role Projection produces exactly one Role.

Derivation Rule has Role Projection.
  Each Derivation Rule has some Role Projection.

Fact Type has Derivation Storage Type.
  Each Fact Type has at most one Derivation Storage Type.

## Antecedent Clause Shape (#281)

<!--
Every clause inside a derivation-rule antecedent should parse into a
recognised `Clause Shape`. If the compiler can't attach a shape (the
clause didn't match any known pattern — Fact-Type literal, Antecedent
Role bind, Negation, Comparison, …) the rule is unsafe to chain and
the validator surfaces the violation. Expressing this as a deontic
constraint lets the runtime emit the diagnostic through Theorem 4's
violation path rather than a hard-coded check pass.
-->

Antecedent Clause(.id) is an entity type.
Clause Shape is a value type.
  The possible values of Clause Shape are 'fact-type-literal', 'antecedent-role', 'negation', 'comparison', 'conjunction', 'quantified', 'unresolved'.

Derivation Rule has Antecedent Clause.
  Each Derivation Rule has some Antecedent Clause.
  For each Antecedent Clause, exactly one Derivation Rule has that Antecedent Clause.

Antecedent Clause has Clause Shape.
  Each Antecedent Clause has at most one Clause Shape.

It is obligatory that each Antecedent Clause has Clause Shape.

It is forbidden that each Noun has a name that ends with 'ies'.

## Migration (#348)

### Rationale
Population-level rewriting when a schema evolves. §5 allows migration
to land as derivation rules / transition triggers / deontic
constraints; none is shaped for "rewrite facts of one Fact Type into
facts of another," so `Migration` names it directly. Firing a rule
emits a `MigrationApplication` (#349); visible_population (#350)
projects out migrated sources, keeping P monotonic.

Migration(.id) is an entity type.
Migration Rule Text is a value type.

Migration has source Fact Type.
  Each Migration has exactly one source Fact Type.

Migration produces target Fact Type.
  Each Migration produces some target Fact Type.

Migration has Migration Rule Text.
  Each Migration has exactly one Migration Rule Text.

Migration has Timestamp.
  Each Migration has exactly one Timestamp.

It is obligatory that each Migration produces some target Fact Type.

## Migration Application (#349)

### Rationale
Migration firing emits a Migration Application per source fact touched,
recording which target facts were produced and when. §5 Theorem 5 holds
because Migration Application is itself a fact: the visible_population
projection (#350) reads it to filter out migrated sources without a
destructive write, so population monotonicity is preserved and Cor 3
(closure under self-modification) survives.

Migration Application(.id) is an entity type.

Migration Application has Migration.
  Each Migration Application has exactly one Migration.

Migration Application has source Fact.
  Each Migration Application has exactly one source Fact.

Migration Application produces Fact.
  Each Migration Application produces some Fact.
  It is possible that some Migration Application produces more than one Fact.

Migration Application has Timestamp.
  Each Migration Application has exactly one Timestamp.

## Migration Application ordering (#351)

<!--
### Rationale
Two deontics compose migration chains and make federation convergence
constructive. The at-most-one obligation rules out direct v1 → v3
shortcuts: competing MAs for the same source row would flag, forcing
the chain through v2 via a paired Migration + MA. The distinct-Timestamp
obligation is what lets two peers replay the same Migration + MA stream
and converge by Cor 5 — timestamps establish the total order the
replay needs, and visible_population is a function of the replayed
set, so partial replays up to any T agree across peers.
-->

It is obligatory that each source Fact has at most one Migration Application per target Fact Type.

It is obligatory that each Migration Application has a distinct Timestamp.

## NORMA Value Domain (#279)

### Entity types

Bound(.id) is an entity type.
Value Range(.id) is an entity type.
Facet(.id) is an entity type.
Value(.id) is an entity type.
Unit(.Name) is an entity type.
Dimension(.Name) is an entity type.
Conceptual Data Type(.code) is an entity type.
Data Type Group(.code) is an entity type.
Format(.Name) is an entity type.
Textual Constraint is a subtype of Constraint.

### Value types

Regex Pattern is a value type.
Lexical Value is a value type.
Alias is a value type.
Length is a value type.
Binary Precision is a value type.
Digit Count is a value type.
Precision is a value type.
Scale is a value type.
JSON Type is a value type.
JSON Format is a value type.
Abstract SQL Type is a value type.

### Fact types

Value is of Noun.
  Each Value is of exactly one Noun.

Value has Lexical Value.
  Each Value has exactly one Lexical Value.

Value Range has lower Bound.
  Each Value Range has at most one lower Bound.

Value Range has upper Bound.
  Each Value Range has at most one upper Bound.

Bound has Value.
  Each Bound has exactly one Value.

Bound has Clusivity.
  Each Bound has exactly one Clusivity.

Noun has Value Range.
  It is possible that more than one Noun has the same Value Range.

Noun has Facet.
  It is possible that more than one Noun has the same Facet.

Facet has Length.
  Each Facet has at most one Length.

Facet has Binary Precision.
  Each Facet has at most one Binary Precision.

Facet has Digit Count.
  Each Facet has at most one Digit Count.

Facet has Regex Pattern.
  Each Facet has at most one Regex Pattern.

Unit has Dimension.
  Each Unit has exactly one Dimension.

Noun is measured in Unit.
  Each Noun is measured in at most one Unit.

Textual Constraint has Text.
  Each Textual Constraint has exactly one Text.

Noun has Alias.
  It is possible that more than one Noun has the same Alias.

Fact Type has Alias.
  It is possible that more than one Fact Type has the same Alias.

Data Type Group has Name.
  Each Data Type Group has exactly one Name.

Conceptual Data Type is in Data Type Group.
  Each Conceptual Data Type is in exactly one Data Type Group.

Noun has Conceptual Data Type.
  Each Noun has at most one Conceptual Data Type.

Noun has Precision.
  Each Noun has at most one Precision.

Noun has Scale.
  Each Noun has at most one Scale.

Conceptual Data Type has JSON Type.
  Each Conceptual Data Type has exactly one JSON Type.

Conceptual Data Type has JSON Format.
  Each Conceptual Data Type has at most one JSON Format.

Conceptual Data Type has Abstract SQL Type.
  Each Conceptual Data Type has exactly one Abstract SQL Type.

Format is built on Conceptual Data Type.
  Each Format is built on exactly one Conceptual Data Type.
  <!-- Format-on-Conceptual-Data-Type (Phase 1). A `Format` is a
       first-class, extensible REFINEMENT layered ON TOP of exactly one
       base Conceptual Data Type. The base CDT supplies the portable
       skeleton — JSON Type, the base JSON Format, and the Abstract SQL
       Type; the Format refines the *presentation* (a narrower JSON
       Format and an optional validation Pattern) without re-deriving the
       skeleton. The legacy widget Formats ('text', 'date', 'boolean',
       'enum') are seeded below as Format instances built on the matching
       CDT leaf. -->

Format has JSON Format.
  Each Format has at most one JSON Format.
  <!-- The Format's own JSON-Schema `format` keyword, REFINING the base
       CDT's JSON Format (`Conceptual Data Type has JSON Format` above).
       Effective JSON Format of a value type = its Format's JSON Format
       when it declares a Format, ELSE its base CDT's JSON Format. -->

Format has Pattern.
  Each Format has at most one Pattern.
  <!-- Optional regex `pattern` keyword for the Format (reuses the core
       `Pattern` value type at :67). Surfaces in the derived JSON Schema
       as `pattern`; absent when the Format imposes no lexical shape. -->

## Instance Facts

### Constraint Types

Constraint Type 'UC' has Name 'Uniqueness'.
Constraint Type 'MC' has Name 'Mandatory'.
Constraint Type 'FC' has Name 'Frequency'.
Constraint Type 'SS' has Name 'Subset'.
Constraint Type 'EQ' has Name 'Equality'.
Constraint Type 'XC' has Name 'Exclusion'.
Constraint Type 'OR' has Name 'InclusiveOr'.
Constraint Type 'XO' has Name 'ExclusiveOr'.
Constraint Type 'IR' has Name 'Irreflexive'.
Constraint Type 'AS' has Name 'Asymmetric'.
Constraint Type 'AT' has Name 'Antisymmetric'.
Constraint Type 'SY' has Name 'Symmetric'.
Constraint Type 'IT' has Name 'Intransitive'.
Constraint Type 'TR' has Name 'Transitive'.
Constraint Type 'AC' has Name 'Acyclic'.
Constraint Type 'VC' has Name 'ValueComparison'.

### Conceptual Data Types (#279)

<!-- NORMA's portable data-type catalog. Each leaf Conceptual Data Type is
classified into exactly one of eight Data Type Groups (text, numeric,
temporal, logical, raw, other, unspecified, userDefined). The `is in`
facts below are the single source of truth for both the leaf codes and
the group membership; the Data Type Group entities are populated by the
group codes those facts reference (no separate Name is carried in P1).
A value type opts into a data type with `The data type of <ValueType>
is <code>.`, which absorbs `conceptualDataType` onto the Noun cell.

The declaration may carry NORMA facets in a trailing clause (#279 P4):
`The data type of Price is decimal with precision 10 and scale 2.` and
`The data type of Code is text with length 50.`. These absorb onto the
Noun cell as `precision` / `scale` (via `Noun has Precision` / `Noun has
Scale` above) and `maxLength` (via the existing `Noun has Max Length`,
which text `length` reuses). Facets parameterize the projected DDL:
`DECIMAL(precision, scale)`, `CHARACTER VARYING(length)`, etc. The
`Facet` entity (with its own `Length` / `Digit Count` / `Binary
Precision`) models per-instance facet rows for a future supertype /
units pass and is independent of these absorbed Noun fields. -->

Data Type Group 'text' has Name 'Text'.
Data Type Group 'numeric' has Name 'Numeric'.
Data Type Group 'temporal' has Name 'Temporal'.
Data Type Group 'logical' has Name 'Logical'.
Data Type Group 'raw' has Name 'Raw'.
Data Type Group 'other' has Name 'Other'.
Data Type Group 'unspecified' has Name 'Unspecified'.
Data Type Group 'userDefined' has Name 'User Defined'.

Conceptual Data Type 'text' is in Data Type Group 'text'.
Conceptual Data Type 'fixedText' is in Data Type Group 'text'.
Conceptual Data Type 'largeText' is in Data Type Group 'text'.
Conceptual Data Type 'smallInteger' is in Data Type Group 'numeric'.
Conceptual Data Type 'integer' is in Data Type Group 'numeric'.
Conceptual Data Type 'largeInteger' is in Data Type Group 'numeric'.
Conceptual Data Type 'unsignedTiny' is in Data Type Group 'numeric'.
Conceptual Data Type 'unsignedSmall' is in Data Type Group 'numeric'.
Conceptual Data Type 'unsigned' is in Data Type Group 'numeric'.
Conceptual Data Type 'unsignedLarge' is in Data Type Group 'numeric'.
Conceptual Data Type 'autoCounter' is in Data Type Group 'numeric'.
Conceptual Data Type 'singleFloat' is in Data Type Group 'numeric'.
Conceptual Data Type 'doubleFloat' is in Data Type Group 'numeric'.
Conceptual Data Type 'decimal' is in Data Type Group 'numeric'.
Conceptual Data Type 'money' is in Data Type Group 'numeric'.
Conceptual Data Type 'uuid' is in Data Type Group 'numeric'.
Conceptual Data Type 'date' is in Data Type Group 'temporal'.
Conceptual Data Type 'time' is in Data Type Group 'temporal'.
Conceptual Data Type 'dateTime' is in Data Type Group 'temporal'.
Conceptual Data Type 'autoTimestamp' is in Data Type Group 'temporal'.
Conceptual Data Type 'boolean' is in Data Type Group 'logical'.
Conceptual Data Type 'yesNo' is in Data Type Group 'logical'.
Conceptual Data Type 'fixedRaw' is in Data Type Group 'raw'.
Conceptual Data Type 'raw' is in Data Type Group 'raw'.
Conceptual Data Type 'largeRaw' is in Data Type Group 'raw'.
Conceptual Data Type 'picture' is in Data Type Group 'raw'.
Conceptual Data Type 'oleObject' is in Data Type Group 'raw'.
Conceptual Data Type 'rowId' is in Data Type Group 'other'.
Conceptual Data Type 'objectId' is in Data Type Group 'other'.
Conceptual Data Type 'unspecified' is in Data Type Group 'unspecified'.
Conceptual Data Type 'userDefined' is in Data Type Group 'userDefined'.

### Formats (#279 P1)

<!-- The legacy widget Formats, seeded as the Format-on-CDT comment above
promises (it promised and never delivered — the quarry's Rust path or stale
store masked the gap; G4 caught it 2026-07-14). 'enum' builds on 'text':
an enum is text-valued, its domain riding Enum Values. -->

Format 'text' is built on Conceptual Data Type 'text'.
Format 'date' is built on Conceptual Data Type 'date'.
Format 'boolean' is built on Conceptual Data Type 'boolean'.
Format 'enum' is built on Conceptual Data Type 'text'.

<!-- JSON-Schema projection of the catalog (#279 P2a). Each leaf carries one
JSON Type (the `type` keyword the OpenAPI / JSON-Schema generator emits
for a value-type property) and, for temporal / binary / uuid leaves, a
JSON Format. These absorb `jsonType` / `jsonFormat` onto the Conceptual
Data Type cell via RMAP, the same way `conceptualDataType` absorbs onto
Noun. The generator's `JsonTypeMappingTable` reads them back; its boot
fallback mirrors this block one-for-one. (SPEC 10.2: this block was bare
prose inside Instance Facts; commented 2026-07-14.) -->

Conceptual Data Type 'text' has JSON Type 'string'.
Conceptual Data Type 'fixedText' has JSON Type 'string'.
Conceptual Data Type 'largeText' has JSON Type 'string'.
Conceptual Data Type 'smallInteger' has JSON Type 'integer'.
Conceptual Data Type 'integer' has JSON Type 'integer'.
Conceptual Data Type 'largeInteger' has JSON Type 'integer'.
Conceptual Data Type 'unsignedTiny' has JSON Type 'integer'.
Conceptual Data Type 'unsignedSmall' has JSON Type 'integer'.
Conceptual Data Type 'unsigned' has JSON Type 'integer'.
Conceptual Data Type 'unsignedLarge' has JSON Type 'integer'.
Conceptual Data Type 'autoCounter' has JSON Type 'integer'.
Conceptual Data Type 'singleFloat' has JSON Type 'number'.
Conceptual Data Type 'doubleFloat' has JSON Type 'number'.
Conceptual Data Type 'decimal' has JSON Type 'number'.
Conceptual Data Type 'money' has JSON Type 'number'.
Conceptual Data Type 'uuid' has JSON Type 'string'.
Conceptual Data Type 'date' has JSON Type 'string'.
Conceptual Data Type 'time' has JSON Type 'string'.
Conceptual Data Type 'dateTime' has JSON Type 'string'.
Conceptual Data Type 'autoTimestamp' has JSON Type 'string'.
Conceptual Data Type 'boolean' has JSON Type 'boolean'.
Conceptual Data Type 'yesNo' has JSON Type 'boolean'.
Conceptual Data Type 'fixedRaw' has JSON Type 'string'.
Conceptual Data Type 'raw' has JSON Type 'string'.
Conceptual Data Type 'largeRaw' has JSON Type 'string'.
Conceptual Data Type 'picture' has JSON Type 'string'.
Conceptual Data Type 'oleObject' has JSON Type 'string'.
Conceptual Data Type 'rowId' has JSON Type 'integer'.
Conceptual Data Type 'objectId' has JSON Type 'integer'.
Conceptual Data Type 'unspecified' has JSON Type 'string'.
Conceptual Data Type 'userDefined' has JSON Type 'string'.

Conceptual Data Type 'uuid' has JSON Format 'uuid'.
Conceptual Data Type 'date' has JSON Format 'date'.
Conceptual Data Type 'time' has JSON Format 'time'.
Conceptual Data Type 'dateTime' has JSON Format 'date-time'.
Conceptual Data Type 'autoTimestamp' has JSON Format 'date-time'.
Conceptual Data Type 'fixedRaw' has JSON Format 'byte'.
Conceptual Data Type 'raw' has JSON Format 'byte'.
Conceptual Data Type 'largeRaw' has JSON Format 'byte'.
Conceptual Data Type 'picture' has JSON Format 'byte'.
Conceptual Data Type 'oleObject' has JSON Format 'byte'.

<!--
SQL/DDL projection of the catalog (#279 P2b). NORMA maps a Conceptual
Data Type to SQL in two stages: first to an abstract SQL type (the
DCIL layer — a SQL-standard "predefined type" name), then to the
vendor type per dialect (`readings/templates/sql-dialects.md`). The
Abstract SQL Type facts below are stage one. They absorb `abstractSqlType`
onto the Conceptual Data Type cell via RMAP, the same way `jsonType`
absorbs (P2a) and `conceptualDataType` absorbs onto Noun (P1). The
generator's `AbstractSqlTypeTable` reads them back; its boot fallback
mirrors this block one-for-one.
-->

<!--
Type-mapping only (P2b): the IDENTITY / auto-increment semantics of
autoCounter / autoTimestamp and the unsigned-range CHECK constraints
of the unsigned* leaves are deferred to a later phase — here they map
to the abstract type that holds their VALUES (autoCounter / rowId /
objectId → INTEGER; the unsigned* leaves → the smallest signed
abstract type that fits, i.e. one width up where needed). uuid maps
to the abstract UUID type; dialects without a native UUID fall back to
CHARACTER in the vendor layer.
-->

Conceptual Data Type 'text' has Abstract SQL Type 'CHARACTER VARYING'.
Conceptual Data Type 'fixedText' has Abstract SQL Type 'CHARACTER'.
Conceptual Data Type 'largeText' has Abstract SQL Type 'CHARACTER LARGE OBJECT'.
Conceptual Data Type 'smallInteger' has Abstract SQL Type 'SMALLINT'.
Conceptual Data Type 'integer' has Abstract SQL Type 'INTEGER'.
Conceptual Data Type 'largeInteger' has Abstract SQL Type 'BIGINT'.
Conceptual Data Type 'unsignedTiny' has Abstract SQL Type 'SMALLINT'.
Conceptual Data Type 'unsignedSmall' has Abstract SQL Type 'INTEGER'.
Conceptual Data Type 'unsigned' has Abstract SQL Type 'BIGINT'.
Conceptual Data Type 'unsignedLarge' has Abstract SQL Type 'BIGINT'.
Conceptual Data Type 'autoCounter' has Abstract SQL Type 'INTEGER'.
Conceptual Data Type 'singleFloat' has Abstract SQL Type 'REAL'.
Conceptual Data Type 'doubleFloat' has Abstract SQL Type 'DOUBLE PRECISION'.
Conceptual Data Type 'decimal' has Abstract SQL Type 'DECIMAL'.
Conceptual Data Type 'money' has Abstract SQL Type 'DECIMAL'.
Conceptual Data Type 'uuid' has Abstract SQL Type 'UUID'.
Conceptual Data Type 'date' has Abstract SQL Type 'DATE'.
Conceptual Data Type 'time' has Abstract SQL Type 'TIME'.
Conceptual Data Type 'dateTime' has Abstract SQL Type 'TIMESTAMP'.
Conceptual Data Type 'autoTimestamp' has Abstract SQL Type 'TIMESTAMP'.
Conceptual Data Type 'boolean' has Abstract SQL Type 'BOOLEAN'.
Conceptual Data Type 'yesNo' has Abstract SQL Type 'BOOLEAN'.
Conceptual Data Type 'fixedRaw' has Abstract SQL Type 'BINARY'.
Conceptual Data Type 'raw' has Abstract SQL Type 'BINARY VARYING'.
Conceptual Data Type 'largeRaw' has Abstract SQL Type 'BINARY LARGE OBJECT'.
Conceptual Data Type 'picture' has Abstract SQL Type 'BINARY VARYING'.
Conceptual Data Type 'oleObject' has Abstract SQL Type 'BINARY VARYING'.
Conceptual Data Type 'rowId' has Abstract SQL Type 'INTEGER'.
Conceptual Data Type 'objectId' has Abstract SQL Type 'INTEGER'.
Conceptual Data Type 'unspecified' has Abstract SQL Type 'CHARACTER VARYING'.
Conceptual Data Type 'userDefined' has Abstract SQL Type 'CHARACTER VARYING'.

### Constraint Types (#747)

<!--
Each Constraint Type code below names one alethic constraint dispatch
arm in `compile.rs`. The Family groups codes that share an evaluation
shape (ring, set-comparison, subset/equality, value, frequency,
uniqueness, mandatory) so tooling — OpenAPI, docs, MCP introspection —
can enumerate the inventory from declared facts instead of reading the
Rust match. AT and ANS share the antisymmetric ring kernel; ANS is
preserved as a separate kind so the alias surfaces as a fact.
-->

Constraint Type 'IR' has Constraint Type Label 'Irreflexive Ring'.
Constraint Type 'IR' has Constraint Type Family 'ring'.
Constraint Type 'AS' has Constraint Type Label 'Asymmetric Ring'.
Constraint Type 'AS' has Constraint Type Family 'ring'.
Constraint Type 'SY' has Constraint Type Label 'Symmetric Ring'.
Constraint Type 'SY' has Constraint Type Family 'ring'.
Constraint Type 'AT' has Constraint Type Label 'Antisymmetric Ring'.
Constraint Type 'AT' has Constraint Type Family 'ring'.
Constraint Type 'IT' has Constraint Type Label 'Intransitive Ring'.
Constraint Type 'IT' has Constraint Type Family 'ring'.
Constraint Type 'TR' has Constraint Type Label 'Transitive Ring'.
Constraint Type 'TR' has Constraint Type Family 'ring'.
Constraint Type 'AC' has Constraint Type Label 'Acyclic Ring'.
Constraint Type 'AC' has Constraint Type Family 'ring'.
Constraint Type 'RF' has Constraint Type Label 'Reflexive Ring'.
Constraint Type 'RF' has Constraint Type Family 'ring'.
Constraint Type 'UC' has Constraint Type Label 'Uniqueness'.
Constraint Type 'UC' has Constraint Type Family 'uniqueness'.
Constraint Type 'MC' has Constraint Type Label 'Mandatory'.
Constraint Type 'MC' has Constraint Type Family 'mandatory'.
Constraint Type 'FC' has Constraint Type Label 'Frequency'.
Constraint Type 'FC' has Constraint Type Family 'frequency'.
Constraint Type 'VC' has Constraint Type Label 'Value'.
Constraint Type 'VC' has Constraint Type Family 'value'.
Constraint Type 'XO' has Constraint Type Label 'Exclusive Or'.
Constraint Type 'XO' has Constraint Type Family 'set-comparison'.
Constraint Type 'XC' has Constraint Type Label 'Exclusion'.
Constraint Type 'XC' has Constraint Type Family 'set-comparison'.
Constraint Type 'OR' has Constraint Type Label 'Inclusive Or'.
Constraint Type 'OR' has Constraint Type Family 'set-comparison'.
Constraint Type 'SS' has Constraint Type Label 'Subset'.
Constraint Type 'SS' has Constraint Type Family 'subset'.
Constraint Type 'EQ' has Constraint Type Label 'Equality'.
Constraint Type 'EQ' has Constraint Type Family 'equality'.

Constraint Type 'DF_pop' has Constraint Type Label 'Deontic Forbidden (population)'.
Constraint Type 'DF_pop' has Constraint Type Family 'deontic'.
Constraint Type 'DF_cwa' has Constraint Type Label 'Deontic Forbidden (closed-world)'.
Constraint Type 'DF_cwa' has Constraint Type Family 'deontic'.
Constraint Type 'DF_owa' has Constraint Type Label 'Deontic Forbidden (open-world)'.
Constraint Type 'DF_owa' has Constraint Type Family 'deontic'.
Constraint Type 'DO_pop' has Constraint Type Label 'Deontic Obligatory (population)'.
Constraint Type 'DO_pop' has Constraint Type Family 'deontic'.
Constraint Type 'DO_obl' has Constraint Type Label 'Deontic Obligatory'.
Constraint Type 'DO_obl' has Constraint Type Family 'deontic'.
Constraint Type 'DO_sender' has Constraint Type Label 'Deontic Obligatory (sender)'.
Constraint Type 'DO_sender' has Constraint Type Family 'deontic'.

### Join Types (NORMA #279)

Join Type 'inner' has Name 'inner'.
Join Type 'outer' has Name 'outer'.
Join Type 'left-outer' has Name 'left-outer'.
Join Type 'right-outer' has Name 'right-outer'.
Join Type 'anti' has Name 'anti'.

### HTTP Methods

HTTP Method 'GET' has Name 'GET'.
HTTP Method 'POST' has Name 'POST'.
HTTP Method 'PUT' has Name 'PUT'.
HTTP Method 'PATCH' has Name 'PATCH'.
HTTP Method 'DELETE' has Name 'DELETE'.
HTTP Method 'HEAD' has Name 'HEAD'.
HTTP Method 'OPTIONS' has Name 'OPTIONS'.

### External Systems

<!-- External System auth shape instance facts (URL/Header/Prefix/Country Code/Kind) for auth.vin, auto.dev, stripe, github, resend live in arest/readings/templates/connectors.md. Per-app Domain Connection facts carrying Secret References live in each consuming app's gitignored .env file. -->

Domain 'core' has Access 'public'.
Domain 'core' has Description 'Extracted from NORMA ORM2 model (design/html/). The canonical FORML 2 metamodel against which every user domain is a subtype binding.'.
