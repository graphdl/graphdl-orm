# AREST State: Behavioral Entities

## Entity Types

Status(.Name) is an entity type.
State Machine Definition is a subtype of Status.
Transition(.id) is an entity type.
Guard(.Name) is an entity type.

Stream(.id) is an entity type.

## Readings

### State Machine Definition
State Machine Definition belongs to Domain.
  Each State Machine Definition belongs to exactly one Domain.
State Machine Definition is for Noun.
  Each State Machine Definition is for exactly one Noun.

### Status
Verb is performed in Status.
  Each Verb is performed in at most one Status.

### Transition
Transition is defined in State Machine Definition.
  Each Transition is defined in exactly one State Machine Definition.
Transition is from Status.
  Each Transition is from exactly one Status.
Transition is to Status.
  Each Transition is to exactly one Status.
Transition is triggered by Fact Type.
  Each Transition is triggered by exactly one Fact Type.
Verb is performed during Transition.
  Each Verb is performed during at most one Transition.

### Status
Status is initial in State Machine Definition.
  Each State Machine Definition has at most one initial Status.
Status is defined in State Machine Definition. *
Status is terminal in State Machine Definition. *
Status is rooted in State Machine Definition. *

### Guard
Guard references Fact Type.
  It is possible that some Guard references more than one Fact Type and that for some Fact Type, more than one Guard references that Fact Type.
  For each combination of Guard and Fact Type, that Guard references that Fact Type at most once.
Guard prevents Transition.
  Each Guard prevents at most one Transition.
  It is possible that more than one Guard prevents the same Transition.

## Derivation Rules

* Status is defined in State Machine Definition iff some Transition is defined in that State Machine Definition and that Transition is from that Status or that Transition is to that Status.

* Status is terminal in State Machine Definition iff that Status is defined in that State Machine Definition and no Transition is defined in that State Machine Definition where that Transition is from that Status.

<!--
  #759 / Audit MC3b-a: normalized SM derivation rules covering Pass 1
  / 2 / 2b of the Rust function `derive_state_machines_from_facts`
  (compile.rs:372-507). Together with the existing transition-driven
  derivation above, these rules let the SM cell be populated from
  instance facts via the engine's forward-chain — no Rust path needed.
  The existing JSON-blob StateMachine cell stays live as fallback
  until #761-#763 swap consumers over and #763 deletes the typed
  StateMachineDef + the Rust function.

    Pass 1 (compile.rs:376-383): instance facts of the form
      `State Machine Definition 'X' is for Noun 'Y'`
    already register the SM record by virtue of the FT itself; no
    derivation rule needed.

    Pass 2 (compile.rs:385-401): an `initial in` declaration entails
    that the same Status is defined in the same SM. The rule below
    fires whenever the parser captures the initial-marking fact,
    populating `Status_is_defined_in_State_Machine_Definition` so
    downstream consumers see initial Statuses without the Rust path.

    Pass 2b (compile.rs:403-415): a non-initial
      `Status 'S' is defined in State Machine Definition 'X'`
    instance fact is a direct assertion the parser already routes
    into the same cell; no derivation rule needed (Status is defined
    in SM is `*` in the FT declaration above for the transition-driven
    derivation, but assertable instance facts still land in the cell
    per the parser's normal instance-fact pathway).
-->

* Status is defined in State Machine Definition iff that Status is initial in that State Machine Definition.

<!--
  #760 / Audit MC3b-b: Pass-4 graph-derived initial Status. Mirrors
  the source-never-target topology fold in
  `derive_state_machines_from_facts` at compile.rs:479-505.

  A Status is "rooted" in a SM iff it is the source of some Transition
  in that SM and no Transition in that SM has it as target. The
  consumer side (#761 — `compile_state_machine`) promotes a single
  rooted Status to `is initial in` ONLY when the rooted set has
  cardinality 1; ambiguity (multiple rooted, zero rooted, or cycles)
  leaves the SM without an inferred initial — the same behaviour the
  Rust path implements at compile.rs:502-504.

  Two things the rule is NOT able to express on its own and that the
  consumer side (#761) must therefore implement:

  (1) Uniqueness gate. FORML 2 derivations are monotonic — "exactly
      one rooted Status per SM" is a cardinality predicate, not a
      join. Per task #760's option (a) we deliberately stop short
      and emit every candidate; the consumer applies cardinality.

  (2) Strict set-difference negation. The parser currently strips
      the leading `no` and the trailing `where …` clause and falls
      back to resolving the bare FT (parse_forml2.rs:1184-1194), so
      the negative antecedent does not produce an `AbsenceOf` source
      — only the explicit `_cwa_negation_…` synthetic rules in
      compile.rs:2546-2599 currently emit AbsenceOf, and only for
      CWA nouns. Until parser-side negation lands as a follow-up,
      this rule over-emits for source-AND-target Statuses; the
      consumer's cardinality gate filters that case naturally
      (over-emit ⇒ |rooted| > 1 ⇒ no initial inferred ⇒ same end
      result as the Rust path's "ambiguous" branch).

  See task #760 report for grammar coverage notes.
-->

* Status is rooted in State Machine Definition iff some Transition is defined in that State Machine Definition and that Transition is from that Status and no Transition is defined in that State Machine Definition where that Transition is to that Status.

## Constraints

For each Noun, at most one State Machine Definition is for that Noun.
Each State Machine Definition has exactly one initial Status.
It is obligatory that each State Machine Definition has at least one terminal Status.
If some Status is initial in some State Machine Definition then that Status is defined in that State Machine Definition.

## Instance Facts

Domain 'state' has Access 'public'.
