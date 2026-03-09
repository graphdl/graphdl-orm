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
StateMachineDefinition belongs to Domain (*:1)
StateMachineDefinition has Title (*:1)
StateMachineDefinition is for Noun (*:1)

### Status
Status belongs to StateMachineDefinition (*:1)
Status has Name (*:1)
Verb is performed in Status (*:1)

### Transition
Transition has Status as source (*:1)
Transition has Status as target (*:1)
Transition is triggered by EventType (*:1)
Verb is performed during Transition (*:1)

### Guard
Guard has Name (*:1)
Guard references GraphSchema (*:1)
Guard prevents Transition (*:1)
