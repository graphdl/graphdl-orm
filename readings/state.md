# GraphDL State — Behavioral Entities

## Entity Types

State Machine Definition(.Title) is an entity type.
Status(.Name) is an entity type.
Transition is an entity type.
Guard(.Name) is an entity type.

Event Type(.id) is an entity type.
Stream(.id) is an entity type.

## Value Types

Title is a value type.

## Readings

### State Machine Definition
State Machine Definition belongs to Domain.
  Each State Machine Definition belongs to exactly one Domain.
State Machine Definition is for Noun.
  Each State Machine Definition is for exactly one Noun.

### Status
Status belongs to State Machine Definition.
  Each Status belongs to exactly one State Machine Definition.
Verb is performed in Status.
  Each Verb is performed in at most one Status.

### Transition
Transition has Status as source.
  Each Transition has exactly one Status as source.
Transition has Status as target.
  Each Transition has exactly one Status as target.
Transition is triggered by Event Type.
  Each Transition is triggered by exactly one Event Type.
Verb is performed during Transition.
  Each Verb is performed during at most one Transition.

### Guard
Guard references Graph Schema.
  It is possible that some Guard references more than one Graph Schema and that for some Graph Schema, more than one Guard references that Graph Schema.
  For each combination of Guard and Graph Schema, that Guard references that Graph Schema at most once.
Guard prevents Transition.
  Each Guard prevents at most one Transition.
  It is possible that more than one Guard prevents the same Transition.

### State Machine Definition
For each State Machine Definition, some Status belongs to that State Machine Definition.
For each Noun, at most one State Machine Definition is for that Noun.
