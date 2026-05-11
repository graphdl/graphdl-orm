# CSDP Authorship

Exercises: schema authorship as a workflow, HATEOAS-guided modeling,
Domain Change preparation, and readings-first self-modification.

## Entity Types

Authoring Session(.Authoring Session Id) is an entity type.
Authoring Step(.Authoring Step Name) is an entity type.
Authoring Tool(.Authoring Tool Name) is an entity type.

## Value Types

Authoring Session Id is a value type.
Authoring Step Name is a value type.
Authoring Tool Name is a value type.
Authoring Step Order is a value type.
Authoring Situation is a value type.
Authoring Guidance is a value type.

## Readings

### Authoring Step

Authoring Step has Authoring Step Order.
  Each Authoring Step has exactly one Authoring Step Order.

Authoring Step applies in Authoring Situation.
  Each Authoring Step applies in exactly one Authoring Situation.

Authoring Step has Authoring Guidance.
  Each Authoring Step has exactly one Authoring Guidance.

Authoring Step recommends Authoring Tool.
  It is possible that some Authoring Step recommends more than one Authoring Tool.
  It is possible that some Authoring Tool is recommended by more than one Authoring Step.
  In each population of Authoring Step recommends Authoring Tool, each Authoring Step, Authoring Tool combination occurs at most once.

Authoring Step uses Status.
  Each Authoring Step uses exactly one Status.
  Each Status is used by at most one Authoring Step.

### Domain Change Authorship

Domain Change is authored through Authoring Session.
  Each Domain Change is authored through at most one Authoring Session.

## Instance Facts

### CSDP Authoring State Machine

State Machine Definition 'CSDP Authoring' is for Noun 'Authoring Session'.
Status 'Inspect Existing Model' is defined in State Machine Definition 'CSDP Authoring'.
Status 'Elicit Example Facts' is defined in State Machine Definition 'CSDP Authoring'.
Status 'Declare Object and Value Types' is defined in State Machine Definition 'CSDP Authoring'.
Status 'Add Elementary Fact Type' is defined in State Machine Definition 'CSDP Authoring'.
Status 'Add Constraints' is defined in State Machine Definition 'CSDP Authoring'.
Status 'Add Derivations and Workflows' is defined in State Machine Definition 'CSDP Authoring'.
Status 'Verbalize and Validate' is defined in State Machine Definition 'CSDP Authoring'.
Status 'Stage or Compile' is defined in State Machine Definition 'CSDP Authoring'.
Status 'Inspect Existing Model' is initial in State Machine Definition 'CSDP Authoring'.
Status 'Stage or Compile' is terminal in State Machine Definition 'CSDP Authoring'.

Transition 'elicit-example-facts' is defined in State Machine Definition 'CSDP Authoring'.
Transition 'elicit-example-facts' is from Status 'Inspect Existing Model'.
Transition 'elicit-example-facts' is to Status 'Elicit Example Facts'.
Transition 'elicit-example-facts' is triggered by Event Type 'elicit-example-facts'.

Transition 'declare-object-and-value-types' is defined in State Machine Definition 'CSDP Authoring'.
Transition 'declare-object-and-value-types' is from Status 'Elicit Example Facts'.
Transition 'declare-object-and-value-types' is to Status 'Declare Object and Value Types'.
Transition 'declare-object-and-value-types' is triggered by Event Type 'declare-object-and-value-types'.

Transition 'add-elementary-fact-type' is defined in State Machine Definition 'CSDP Authoring'.
Transition 'add-elementary-fact-type' is from Status 'Declare Object and Value Types'.
Transition 'add-elementary-fact-type' is to Status 'Add Elementary Fact Type'.
Transition 'add-elementary-fact-type' is triggered by Event Type 'add-elementary-fact-type'.

Transition 'add-constraints' is defined in State Machine Definition 'CSDP Authoring'.
Transition 'add-constraints' is from Status 'Add Elementary Fact Type'.
Transition 'add-constraints' is to Status 'Add Constraints'.
Transition 'add-constraints' is triggered by Event Type 'add-constraints'.

Transition 'add-derivations-and-workflows' is defined in State Machine Definition 'CSDP Authoring'.
Transition 'add-derivations-and-workflows' is from Status 'Add Constraints'.
Transition 'add-derivations-and-workflows' is to Status 'Add Derivations and Workflows'.
Transition 'add-derivations-and-workflows' is triggered by Event Type 'add-derivations-and-workflows'.

Transition 'verbalize-and-validate' is defined in State Machine Definition 'CSDP Authoring'.
Transition 'verbalize-and-validate' is from Status 'Add Derivations and Workflows'.
Transition 'verbalize-and-validate' is to Status 'Verbalize and Validate'.
Transition 'verbalize-and-validate' is triggered by Event Type 'verbalize-and-validate'.

Transition 'stage-or-compile' is defined in State Machine Definition 'CSDP Authoring'.
Transition 'stage-or-compile' is from Status 'Verbalize and Validate'.
Transition 'stage-or-compile' is to Status 'Stage or Compile'.
Transition 'stage-or-compile' is triggered by Event Type 'stage-or-compile'.

### CSDP Authoring Steps

Authoring Step 'inspect-existing-model' has Authoring Step Order '1'.
Authoring Step 'inspect-existing-model' applies in Authoring Situation 'Before adding or changing readings'.
Authoring Step 'inspect-existing-model' has Authoring Guidance 'Find existing nouns, fact types, constraints, derivations, and state machines so the change extends the current UoD instead of duplicating it'.
Authoring Step 'inspect-existing-model' recommends Authoring Tool 'schema'.
Authoring Step 'inspect-existing-model' recommends Authoring Tool 'query'.
Authoring Step 'inspect-existing-model' recommends Authoring Tool 'tutor'.
Authoring Step 'inspect-existing-model' uses Status 'Inspect Existing Model'.

Authoring Step 'elicit-example-facts' has Authoring Step Order '2'.
Authoring Step 'elicit-example-facts' applies in Authoring Situation 'When app behavior is still described as a feature request'.
Authoring Step 'elicit-example-facts' has Authoring Guidance 'Collect concrete population examples and verbalize them as elementary facts before declaring abstractions'.
Authoring Step 'elicit-example-facts' recommends Authoring Tool 'tutor'.
Authoring Step 'elicit-example-facts' uses Status 'Elicit Example Facts'.

Authoring Step 'declare-object-and-value-types' has Authoring Step Order '3'.
Authoring Step 'declare-object-and-value-types' applies in Authoring Situation 'When examples reveal a thing with independent identity or a scalar identifying value'.
Authoring Step 'declare-object-and-value-types' has Authoring Guidance 'Choose entity types for referenced things and value types for printed or compared values, then add reference schemes before dependent facts need identity'.
Authoring Step 'declare-object-and-value-types' recommends Authoring Tool 'tutor'.
Authoring Step 'declare-object-and-value-types' recommends Authoring Tool 'propose'.
Authoring Step 'declare-object-and-value-types' recommends Authoring Tool 'compile'.
Authoring Step 'declare-object-and-value-types' uses Status 'Declare Object and Value Types'.

Authoring Step 'add-elementary-fact-type' has Authoring Step Order '4'.
Authoring Step 'add-elementary-fact-type' applies in Authoring Situation 'When a natural domain sentence is missing from the model'.
Authoring Step 'add-elementary-fact-type' has Authoring Guidance 'Write the natural FORML2 reading in the domain expert phrasing and preserve natural arity instead of forcing binary fields'.
Authoring Step 'add-elementary-fact-type' recommends Authoring Tool 'tutor'.
Authoring Step 'add-elementary-fact-type' recommends Authoring Tool 'propose'.
Authoring Step 'add-elementary-fact-type' recommends Authoring Tool 'compile'.
Authoring Step 'add-elementary-fact-type' uses Status 'Add Elementary Fact Type'.

Authoring Step 'add-constraints' has Authoring Step Order '5'.
Authoring Step 'add-constraints' applies in Authoring Situation 'After the fact type reads correctly with sample population'.
Authoring Step 'add-constraints' has Authoring Guidance 'Add uniqueness, mandatory, value, subset, exclusion, ring, subtype, and deontic or alethic constraints required by the examples'.
Authoring Step 'add-constraints' recommends Authoring Tool 'tutor'.
Authoring Step 'add-constraints' recommends Authoring Tool 'propose'.
Authoring Step 'add-constraints' recommends Authoring Tool 'compile'.
Authoring Step 'add-constraints' uses Status 'Add Constraints'.

Authoring Step 'add-derivations-and-workflows' has Authoring Step Order '6'.
Authoring Step 'add-derivations-and-workflows' applies in Authoring Situation 'When a fact follows from other facts or an entity has a lifecycle'.
Authoring Step 'add-derivations-and-workflows' has Authoring Guidance 'Prefer derivation rules for computable facts and state-machine facts for statuses, events, triggers, guards, and available actions'.
Authoring Step 'add-derivations-and-workflows' recommends Authoring Tool 'tutor'.
Authoring Step 'add-derivations-and-workflows' recommends Authoring Tool 'actions'.
Authoring Step 'add-derivations-and-workflows' recommends Authoring Tool 'propose'.
Authoring Step 'add-derivations-and-workflows' recommends Authoring Tool 'compile'.
Authoring Step 'add-derivations-and-workflows' uses Status 'Add Derivations and Workflows'.

Authoring Step 'verbalize-and-validate' has Authoring Step Order '7'.
Authoring Step 'verbalize-and-validate' applies in Authoring Situation 'Before mutating the active app schema'.
Authoring Step 'verbalize-and-validate' has Authoring Guidance 'Check that the candidate readings still read naturally, parse cleanly, and preserve constraints against examples'.
Authoring Step 'verbalize-and-validate' recommends Authoring Tool 'tutor'.
Authoring Step 'verbalize-and-validate' recommends Authoring Tool 'tutor.compile'.
Authoring Step 'verbalize-and-validate' recommends Authoring Tool 'propose'.
Authoring Step 'verbalize-and-validate' uses Status 'Verbalize and Validate'.

Authoring Step 'stage-or-compile' has Authoring Step Order '8'.
Authoring Step 'stage-or-compile' applies in Authoring Situation 'Only after the CSDP step is explicit and validated'.
Authoring Step 'stage-or-compile' has Authoring Guidance 'Governed schema evolution goes through propose and immediate self-modification goes through compile'.
Authoring Step 'stage-or-compile' recommends Authoring Tool 'propose'.
Authoring Step 'stage-or-compile' recommends Authoring Tool 'compile'.
Authoring Step 'stage-or-compile' uses Status 'Stage or Compile'.

Domain 'authoring' has Visibility 'public'.
