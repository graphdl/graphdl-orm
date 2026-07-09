# AREST Instances: Runtime Entities

## Entity Types

Resource(.Reference) is an entity type.
  Resource is a subtype of Noun.
Event is an entity type.
  Event is a subtype of Resource.
Fact is an entity type.
  Fact is a subtype of Event.
State Machine(.id) is an entity type.
Guard Run(.Name) is an entity type.
Citation(.id) is an entity type.
User(.Email) is an entity type.

## Value Types

Reference is a value type.
Email is a value type.
Value is a value type.
Retrieval Date is a value type.

Cell Name is a value type.
Cell Version Id is a value type.

Authority Type is a value type.
  The possible values of Authority Type are 'Constitutional', 'Statute', 'Regulation', 'Case', 'Rule-of-Court', 'Executive-Order', 'Treaty', 'Agency-Guidance', 'Industry-Standard', 'Administrative-Ruling', 'Runtime-Function', 'Federated-Fetch', 'Storage-Pin'.

## Readings

### Citation
Citation has Text.
  Each Citation has exactly one Text.
Citation has URI.
  Each Citation has at most one URI.
Citation has Retrieval Date.
  Each Citation has at most one Retrieval Date.
Citation has Authority Type.
  Each Citation has at most one Authority Type.
  It is possible that more than one Citation has the same Authority Type.
Citation is backed by External System.
  Each Citation is backed by at most one External System.
  It is possible that more than one Citation is backed by the same External System.
Citation pins Cell Name.
  Each Citation pins at most one Cell Name.
  It is possible that more than one Citation pins the same Cell Name.
Citation pins Cell Version Id.
  Each Citation pins at most one Cell Version Id.
  It is possible that more than one Citation pins the same Cell Version Id.

### Fact
Fact is of Fact Type.
  Each Fact is of exactly one Fact Type.
Fact is of Function. *
  <!-- ns-2 (ns-derive-population-domains): the single-sourcing BRIDGE for a
       Fact's domain. A Fact Type IS a Function (Fact Type < Resource < Noun <
       Function; same identity, same id), so this fully-derived FT re-labels
       the Fact Type a Fact is of as that same Function. It stores NO domain —
       it only re-projects the existing `Fact is of Fact Type` value under a
       `Function` role so the domain rule below can JOIN on `Function` (the
       role `Function belongs to Domain` carries), exactly as Violation/Failure
       join on the Function they are against. See the rule under
       "## Derivation Rules". -->
Fact belongs to Domain. *
  <!-- ns-2 (ns-derive-population-domains): a Fact does NOT store its own
       domain — it DERIVES it from its Fact Type, keeping domain single-sourced
       on Function (a Fact Type is a subtype of Function via Resource < Noun <
       Function; core.md "Function belongs to Domain"). The `*` marks this Fact
       Type as fully derived; the rule is under "## Derivation Rules" below.
       Fact never stored a domain, so nothing is removed — this only adds the
       derivation so a Fact's domain is the domain of its Fact Type. -->
Fact is completed.
Fact is example.
Fact cites Citation.
  For each pair of Fact and Citation, that Fact cites that Citation at most once.

### Event
Event is of Event Type.
  Each Event is of exactly one Event Type.
Event occurred at Timestamp.
  Each Event occurred at exactly one Timestamp.
Event is created by State Machine.
  Each Event is created by at most one State Machine.
  It is possible that more than one Event is created by the same State Machine.

### Event Type
Event Type publishes to Stream.
  Each Event Type publishes to at most one Stream.
  It is possible that more than one Event Type publishes to the same Stream.
Event Type can be created by Verb.
  It is possible that some Event Type can be created by more than one Verb and that some Verb can create more than one Event Type.
  For each combination of Event Type and Verb, that Event Type can be created by that Verb at most once.

### Fact Type Citation
Fact Type cites Citation.
  For each pair of Fact Type and Citation, that Fact Type cites that Citation at most once.
  It is possible that some Fact Type cites more than one Citation.
  It is possible that more than one Fact Type cites the same Citation.

### Resource
Resource is instance of Noun.
  Each Resource is instance of some Noun.
<!-- 'exactly one Noun' was NON-CANONICAL (challenged 2026-07-09, verified
     against Halpin, "Subtyping Revisited", NORMA): in ORM subtyping is
     population inclusion — "all instances of one type are also instances
     of a more encompassing type" — so an entity is legitimately an
     instance of its subtype AND every supertype (Patient 101 is in both
     the MalePatient and Patient populations). Membership is transitive;
     there is no single type per entity. The old uniqueness fired alethic
     on every subtype/multi-typed id on recompile. Mandatory only now
     (every Resource has some type). 'inherited' in ORM is PROPERTY reuse
     (a subtype plays the supertype's roles because it IS a supertype
     instance), not a separate membership relation — so the over-broad
     mirror (instance of every role-noun, supertypes included) is the
     CORRECT transitive membership, and 'Resource is inherited instance
     of Noun' is a non-canonical crutch (retire separately). -->

Resource is of Function. *
  <!-- ns-2 (ns-derive-population-domains): the single-sourcing BRIDGE for a
       Resource's domain. A Noun IS a Function (Noun < Function; same identity,
       same id), so this fully-derived FT re-labels the Noun a Resource is an
       instance of as that same Function. It stores NO domain — it only
       re-projects the existing `Resource is instance of Noun` value under a
       `Function` role so the domain rule below can JOIN on `Function` (the
       role `Function belongs to Domain` carries), exactly as Violation/Failure
       join on the Function they are against. See the rule under
       "## Derivation Rules". -->
Resource belongs to Domain. *
  <!-- ns-2 (ns-derive-population-domains): a Resource does NOT store its own
       domain — it DERIVES it from the Noun it is an instance of, keeping
       domain single-sourced on Function (a Noun is a subtype of Function;
       core.md "Function belongs to Domain"). The `*` marks this Fact Type as
       fully derived; the rule is under "## Derivation Rules" below. Resource
       never stored a domain, so nothing is removed — this only adds the
       derivation so a Resource's domain is the domain of its Noun. Distinct
       from the createEntity `domain` command field (ast.rs `same_identity` /
       `annotate_noun_domain`), which is the per-FILE namespace tag, not a
       stored population-level domain fact. -->
Resource has Reference.
  Each Resource has at most one Reference.
Resource has Value.
  Each Resource has at most one Value.
Resource is created by User.
  Each Resource is created by at most one User.

### Resource Role
Fact uses Resource for Role.
  Each Fact uses at most one Resource for each Role.
  Each Fact uses some Resource for some Role.
This association with Fact, Resource, Role provides the preferred identification scheme for Resource Role.

### State Machine (runtime instance of State Machine Definition)
State Machine is instance of State Machine Definition.
  Each State Machine is instance of exactly one State Machine Definition.
State Machine is instance of Noun.
  Each State Machine is instance of exactly one Noun.
<!-- task-987 / junk-writer-3: the SM seed has ALWAYS written this
     triple at runtime (compile.rs sm seed: instance_of_Noun +
     for_Resource + currently_in_Status), but the fact type was never
     DECLARED — so cor:closure's orphan GC dropped the population at
     every compile and the next SM init re-minted it, forever
     (arc-agi-3 issue-13 forensics). Declaring the engine's own
     vocabulary makes the population legal, persistent, queryable
     (a 3NF table), and subject to validate — the substrate-derived
     987 ruling: complete the self-description, never scope it. -->
State Machine is for Resource.
  Each Resource has at most one State Machine.
State Machine is for Resource. *

<!-- [REMOVED 2026-06-12, board-derived-layer poisoning] The rule
     `* State Machine is for Resource iff Resource is instance of Noun
     and some State Machine Definition is for that Noun.` is
     UNDERSPECIFIED: it cannot bind WHICH State Machine, so it emitted
     one-role partial tuples. Starved for months (its antecedent
     `Resource is instance of Noun` was empty), it activated the moment
     the task-987 membership reflection populated that cell — the
     partials landed first in the Resource-keyed cell and
     KeyConflict-displaced every real SM-for-Resource fact, emptying
     the entire Task derived layer downstream (status bridge,
     recommendation markers). The REAL writers are the SM seed
     (compile.rs s0 trio) and the task-929 for-Resource backfill; the
     chain-side arity-completeness guard is the defense-in-depth. -->


### State (projected from SM via State Machine is for Resource × State Machine is currently in Status)
<!-- task-742 rename context: post-rename the canonical SM status
     lives in State_Machine_is_currently_in_Status, keyed by the
     SM entity id; the per-Resource projection materialises via the
     SM-for-Resource role chain. Resource is an abstract noun so
     RMAP cannot absorb the status into a Resource cell -- there
     IS no Resource cell. App-level readings (e.g. apps/tasks/
     readings/app.md) carry the explicit projection
     "Resource is currently in Status iff some State Machine is
     for that Resource and that State Machine is currently in
     that Status."  -->
Resource is currently in Status.
  Each Resource is currently in at most one Status.

# task-955/924: key the SM-keyed status projection so it stays single-valued.
# The engine's imperative transition write AND the SM event-fold both write
# `State_Machine_is_currently_in_Status`; without this UC the cell is un-keyed,
# so the chain folds it by full tuple and the event-fold (which emits one
# status per triggered event) ACCUMULATES every historical status — the
# 923/924 readback artifact. Keyed by State Machine, integrate_round_facts'
# keyed-upsert collapses the per-resource emits to last-write-wins (the latest
# transition target, in transition_table declaration order).
State Machine is currently in Status.
  Each State Machine is currently in exactly one Status.

### Event Caused Transition (objectification of "Event caused Transition in State Machine")
Event caused Transition in State Machine.
  In each population of Event caused Transition in State Machine, each Event, Transition, State Machine combination occurs at most once.
  This association with Event, Transition, State Machine provides the preferred identification scheme for Event Caused Transition.

## Subset Constraints

If some Event caused some Transition in some State Machine then that Event is of some Event Type
  where that Transition is triggered by that Event Type.

### Guard Run
Guard Run is for Guard.
  Each Guard Run is for exactly one Guard.
Guard Run references Fact.
  It is possible that some Guard Run references more than one Fact and that some Fact is referenced by more than one Guard Run.
  For each combination of Guard Run and Fact, that Guard Run references that Fact at most once.
Guard Run has Result.
  Each Guard Run has at most one Result.

## Derivation Rules

<!-- ns-2 (ns-derive-population-domains): instance-level populations DERIVE
     their home Domain from their associated Function-subtype, keeping domain
     single-sourced on Function (core.md "Function belongs to Domain"). These
     rules FIRE through forward-chain (mirroring the outcomes.md Violation /
     Failure rules), via a single-sourcing FUNCTION BRIDGE.

     WHY THE BRIDGE: the forward-chaining JOIN that propagates the Domain value
     only forms when the relating Fact Type and `belongs to Domain` share a
     role NOUN-NAME, and the relating clause must resolve to a declared FT (the
     SchemaCatalog is keyed by role noun-SET). The outcomes rules join directly
     because `is against Function` and `Function belongs to Domain` both carry a
     `Function` role. A Resource's natural relating fact is `is instance of
     Noun` and a Fact's is `is of Fact Type` — those carry a `Noun` / `Fact
     Type` role, and a clause `that Noun belongs to Domain` neither resolves to
     the Function-keyed `Function belongs to Domain` FT (noun-set `[Domain,
     Noun]` is not declared) nor shares its `Function` role, so no join forms.

     A Noun IS a Function and a Fact Type IS a Function (Fact Type < Resource <
     Noun < Function), with the SAME identity / id. So the fully-derived bridge
     FTs `Resource is of Function` / `Fact is of Function` (declared above)
     re-label that same value under a `Function` role — a 1-antecedent
     ModusPonens with a computed-binding rename (`Function is Noun` /
     `Function is Fact Type`); they STORE NO domain. The domain rules then relate
     via `is of Function` and JOIN on `Function` with `Function belongs to
     Domain` — byte-for-byte the Violation / Failure shape — and the Domain
     value propagates onto the Resource / Fact consequent. Domain stays
     single-sourced on Function throughout (the only Domain-valued fact lives on
     the Function; Resource / Fact / the bridge store none).

     A future engine fix (filed task `derivation-subtype-join-resolution`) that
     resolves a subtype-subject `belongs to Domain` clause to the Function FT
     and widens the join across the subtype lattice would let the relating
     clause be the natural `is instance of Noun` / `is of Fact Type` directly
     and retire the bridge; until then the bridge is the readings-only form
     that materialises the single-sourced domain. -->

<!-- DISABLED (convergence-cycle test 2026-06-22; root: absorbed-Function self-reference, see task derivation-subtype-join-resolution):
* Resource is of Function iff Resource is instance of Noun and Function is Noun. -->

<!-- DISABLED (convergence-cycle test 2026-06-22):
* Resource belongs to Domain iff Resource is of Function and that Function belongs to Domain. -->

<!-- `Fact is of Fact Type` is a BASE fact type, populated by population
     reflection (compile.rs `reflect_schema_cells`) — every populated row of a
     fact-type cell IS a Fact of that fact type, the instance-object mirror of
     `Resource is instance of Noun`. It is VALIDATED, not derived, by the
     subset constraint in core.md (`If some Fact uses some Resource for some
     Role then that Fact is of some Fact Type that has that Role`). -->
<!-- DISABLED (convergence-cycle test 2026-06-22):
* Fact is of Function iff Fact is of Fact Type and Function is Fact Type. -->

<!-- DISABLED (convergence-cycle test 2026-06-22):
* Fact belongs to Domain iff Fact is of Function and that Function belongs to Domain. -->

<!-- sm-retire-forml2: SM/Resource status projections lifted from imperative
     Rust into reading-level derivations.

     (1) instance-of-definition (noun-scoped): replaces the compile-time-baked
         definition_id / compile_sm_instance_of_definition_backfill_for. A State
         Machine is an instance of the SM Definition that governs the Noun its
         Resource is an instance of. 3-antecedent equi-join, derivation.md
         shape 6. NOTE: `State Machine is for Resource` (the WHICH-SM binding)
         is NOT lifted — the `same entity as` identity equi-binding the safe
         rule would need is NOT supported by the parser (verified: no
         `same entity as` lowering in parse_forml2), and the underspecified
         partial-tuple form was REMOVED 2026-06-12 for KeyConflict-displacing
         real facts (instances.md 155-167). So compile_sm_for_resource_backfill_for
         is RETAINED.

     (2) Resource-is-currently-in-Status projection: lifts the per-app
         projection (apps/tasks/readings/app.md) to core, replacing the
         imperative Resource_is_currently_in_Status maintenance block in
         command.rs. 2-antecedent equi-join (derivation.md shape 6),
         single-valued.

     (3) SM seed branch: a fresh / eventless SM instance sits at its
         Definition's effective-initial status. 1-antecedent + effective-initial
         join. This is the ONLY authorable half of the current-status producer;
         the ADVANCE branch (last-applicable-transition fold over the
         occurred-ordered trigger-event history) has NO authorable shape (no
         ordered-foldl-into-the-same-cell antecedent in derivation.md 255-368)
         AND requires derived-cell retraction, so the live advance fold
         (compile_sm_reconstruction_fold) stays engine-synthesised. The keyed UC
         on State_Machine_is_currently_in_Status (instances.md 184-193) collapses
         the seed emit and the fold emits to last-write-wins. -->

* State Machine is instance of State Machine Definition iff that State Machine is for some Resource and that Resource is instance of some Noun and that State Machine Definition is for that Noun.

* Resource is currently in Status iff some State Machine is for that Resource and that State Machine is currently in that Status.

* State Machine is currently in Status iff that State Machine is instance of some State Machine Definition and that Status is effective initial in that State Machine Definition.

## Instance Facts

Domain 'instances' has Access 'public'.
