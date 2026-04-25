# AREST State: Behavioral Entities

## Entity Types

Status(.Name) is an entity type.
State Machine Definition is a subtype of Status.
Transition(.id) is an entity type.
Guard(.Name) is an entity type.

Event Type(.id) is an entity type.
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
Transition is triggered by Event Type.
  Each Transition is triggered by exactly one Event Type.
Verb is performed during Transition.
  Each Verb is performed during at most one Transition.

### Event Type
Event Type has Pattern.
  Each Event Type has at most one Pattern.

### Status
Status is initial in State Machine Definition.
  Each State Machine Definition has at most one initial Status.
Status is defined in State Machine Definition. *
Status is terminal in State Machine Definition. *

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

## Constraints

For each Noun, at most one State Machine Definition is for that Noun.
Each State Machine Definition has exactly one initial Status.
It is obligatory that each State Machine Definition has at least one terminal Status.
If some Status is initial in some State Machine Definition then that Status is defined in that State Machine Definition.

## Instance Facts

Domain 'state' has Access 'public'.
