# GraphDL State — Behavioral Entities

## Entity Types

| Entity | Reference Scheme |
|--------|-----------------|
| StateMachineDefinition | Title (within Domain) |
| Status | Name (within StateMachineDefinition) |
| Transition | (within StateMachineDefinition) |
| Guard | Name (within Transition) |

## Readings

### StateMachineDefinition
StateMachineDefinition belongs to Domain.
  Each StateMachineDefinition belongs to at most one Domain.
StateMachineDefinition has Title.
  Each StateMachineDefinition has at most one Title.
StateMachineDefinition is for Noun.
  Each StateMachineDefinition is for at most one Noun.

### Status
Status belongs to StateMachineDefinition.
  Each Status belongs to at most one StateMachineDefinition.
Status has Name.
  Each Status has at most one Name.
Verb is performed in Status.
  Each Verb is performed in at most one Status.

### Transition
Transition has Status as source.
  Each Transition has at most one Status as source.
Transition has Status as target.
  Each Transition has at most one Status as target.
Transition is triggered by EventType.
  Each Transition is triggered by at most one EventType.
Verb is performed during Transition.
  Each Verb is performed during at most one Transition.

### Guard
Guard has Name.
  Each Guard has at most one Name.
Guard references GraphSchema.
  Each Guard references at most one GraphSchema.
Guard prevents Transition.
  Each Guard prevents at most one Transition.
