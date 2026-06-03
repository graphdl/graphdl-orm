# AREST Instances: Runtime Entities

## Entity Types

Resource(.Reference) is an entity type.
  Resource is a subtype of Noun.
Fact is an entity type.
  Fact is a subtype of Fact Type.
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

### Fact Type Citation
Fact Type cites Citation.
  For each pair of Fact Type and Citation, that Fact Type cites that Citation at most once.
  It is possible that some Fact Type cites more than one Citation.
  It is possible that more than one Fact Type cites the same Citation.

### Resource
Resource is instance of Noun.
  Each Resource is instance of exactly one Noun.
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
State Machine is for Resource.
  Each Resource has at most one State Machine.
State Machine is for Resource. *

* State Machine is for Resource iff Resource is instance of Noun and some State Machine Definition is for that Noun.

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

### Fact Triggered Transition (objectification of "Fact triggered Transition for Resource")
Fact triggered Transition for Resource.
  In each population of Fact triggered Transition for Resource, each Fact, Transition, Resource combination occurs at most once.
  This association with Fact, Transition, Resource provides the preferred identification scheme for Fact Triggered Transition.

## Subset Constraints

If some Fact triggered some Transition for some Resource then that Fact is of some Fact Type
  where that Transition is triggered by that Fact Type.

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
     single-sourced on Function (core.md "Function belongs to Domain"). The
     shape is the standard existential-over-join derivation
     (`<X> belongs to Domain iff <X> <relates to> some <Y> and <Y> belongs to
     Domain`), mirroring the outcomes.md Violation / Failure rules.

     PARSER NOTE (gap — same family as core.md's "Implicit Derivation Rules"):
     The forward-chaining JOIN that propagates the Domain value only forms when
     the relating Fact Type and `belongs to Domain` share a role NOUN-NAME. The
     outcomes rules join because `is against Function` and `Function belongs to
     Domain` both carry a `Function` role. These two rules relate via the
     subtype noun (`Noun`, `Fact Type`), and `belongs to Domain` is declared on
     the supertype `Function`, so `resolve_derivation_rule`
     (crates/arest/src/parse_forml2.rs) does not yet resolve a subtype-subject
     `<Noun|Fact Type> belongs to Domain` clause to the Function FT, nor widen
     `is instance of Function` to `is instance of Noun`. Until that subtype-
     subject resolution lands, these rules document the single-sourcing intent
     (Resource ← its Noun's domain; Fact ← its Fact Type's domain) but do not
     yet materialise facts — exactly the documented-but-deferred status of the
     structural rules in core.md §"Implicit Derivation Rules". They store NO
     domain on Resource / Fact regardless. -->

* Resource belongs to Domain iff Resource is instance of Noun and that Noun belongs to Domain.

* Fact belongs to Domain iff Fact is of Fact Type and that Fact Type belongs to Domain.

## Instance Facts

Domain 'instances' has Access 'public'.
