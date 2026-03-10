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
  Each Graph belongs to at most one Domain.
Graph is of GraphSchema.
  Each Graph is of at most one GraphSchema.

### Resource
Resource belongs to Domain.
  Each Resource belongs to at most one Domain.
Resource is instance of Noun.
  Each Resource is instance of at most one Noun.
Resource has Reference.
  Each Resource has at most one Reference.
Resource has Value.
  Each Resource has at most one Value.

### ResourceRole
Graph uses Resource for Role — UC(Graph, Role).

### StateMachine
StateMachine belongs to Domain.
  Each StateMachine belongs to at most one Domain.
StateMachine has Name.
  Each StateMachine has at most one Name.
StateMachine is instance of StateMachineDefinition.
  Each StateMachine is instance of at most one StateMachineDefinition.
StateMachine is currently in Status.
  Each StateMachine is currently in at most one Status.
StateMachine is for Resource.
  Each StateMachine is for at most one Resource.

### Event
Event belongs to StateMachine.
  Each Event belongs to at most one StateMachine.
Event is of EventType.
  Each Event is of at most one EventType.
Event occurred at Timestamp.
  Each Event occurred at at most one Timestamp.
Event is created by Graph.
  Each Event is created by at most one Graph.

### GuardRun
GuardRun has Name.
  Each GuardRun has at most one Name.
GuardRun is for Guard.
  Each GuardRun is for at most one Guard.
GuardRun references Graph.
  Each GuardRun references at most one Graph.
