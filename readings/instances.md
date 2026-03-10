# GraphDL Instances — Runtime Entities

## Entity Types

| Entity | Reference Scheme |
|--------|-----------------|
| Graph | (within Domain) |
| Resource | Reference (within Domain) |
| ResourceRole | (within Graph) |
| StateMachine | Name (within Domain) |
| Event | Timestamp (within StateMachine) |
| GuardRun | Name (within Event) |

## Readings

### Graph
Graph belongs to Domain.
  Each Graph belongs to exactly one Domain.
Graph is of GraphSchema.
  Each Graph is of exactly one GraphSchema.

### Resource
Resource belongs to Domain.
  Each Resource belongs to exactly one Domain.
Resource is instance of Noun.
  Each Resource is instance of exactly one Noun.
Resource has Reference.
  Each Resource has at most one Reference.
Resource has Value.
  Each Resource has at most one Value.

### ResourceRole
Graph uses Resource for Role.
  Each Graph uses at most one Resource for each Role.

### StateMachine
StateMachine belongs to Domain.
  Each StateMachine belongs to exactly one Domain.
StateMachine has Name.
  Each StateMachine has exactly one Name.
StateMachine is instance of StateMachineDefinition.
  Each StateMachine is instance of exactly one StateMachineDefinition.
StateMachine is currently in Status.
  Each StateMachine is currently in exactly one Status.
StateMachine is for Resource.
  Each StateMachine is for at most one Resource.

### Event
Event belongs to StateMachine.
  Each Event belongs to exactly one StateMachine.
Event is of EventType.
  Each Event is of exactly one EventType.
Event occurred at Timestamp.
  Each Event occurred at exactly one Timestamp.
Event is created by Graph.
  Each Event is created by at most one Graph.

### GuardRun
GuardRun has Name.
  Each GuardRun has at most one Name.
GuardRun is for Guard.
  Each GuardRun is for exactly one Guard.
GuardRun references Graph.
  Each GuardRun references at most one Graph.
