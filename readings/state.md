# GraphDL State — Behavioral Entities

## Entity Types

State Machine Definition(.Title) is an entity type.
Status(.Name, .State Machine Definition) is an entity type.
Transition is an entity type.
Guard(.Name) is an entity type.

Event Type(.id) is an entity type.
Stream(.id) is an entity type.

## Value Types

Title is a value type.
Pattern is a value type.

## Readings

### State Machine Definition
State Machine Definition belongs to Domain.
  Each State Machine Definition belongs to exactly one Domain.
State Machine Definition is for Noun.
  Each State Machine Definition is for exactly one Noun.

### Status
Status is defined in State Machine Definition.
  Each Status is defined in exactly one State Machine Definition.
Verb is performed in Status.
  Each Verb is performed in at most one Status.

### Transition
Transition is from Status.
  Each Transition is from exactly one Status.
Transition is to Status.
  Each Transition is to exactly one Status.
Transition is triggered by Event Type.
  Each Transition is triggered by exactly one Event Type.
Verb is performed during Transition.
  Each Verb is performed during at most one Transition.

### Event Type
Event Type has Pattern.
  Each Event Type has at most one Pattern.

### Status
Status is initial.

### Guard
Guard references Graph Schema.
  It is possible that some Guard references more than one Graph Schema and that for some Graph Schema, more than one Guard references that Graph Schema.
  For each combination of Guard and Graph Schema, that Guard references that Graph Schema at most once.
Guard prevents Transition.
  Each Guard prevents at most one Transition.
  It is possible that more than one Guard prevents the same Transition.

## Constraints

For each Status, some Transition is from that Status or some Transition is to that Status.
For each State Machine Definition, some Status is defined in that State Machine Definition.
For each Noun, at most one State Machine Definition is for that Noun.

## Instance Facts

Domain 'state' has Visibility 'public'.
