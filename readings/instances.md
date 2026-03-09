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
Graph belongs to Domain (*:1)
Graph is of GraphSchema (*:1)

### Resource
Resource belongs to Domain (*:1)
Resource is instance of Noun (*:1)
Resource has Reference (*:1)
Resource has Value (*:1)

### ResourceRole
Graph uses Resource for Role — UC(Graph, Role)

### StateMachine
StateMachine belongs to Domain (*:1)
StateMachine has Name (*:1)
StateMachine is instance of StateMachineDefinition (*:1)
StateMachine is currently in Status (*:1)
StateMachine is for Resource (*:1)

### Event
Event belongs to StateMachine (*:1)
Event is of EventType (*:1)
Event occurred at Timestamp (*:1)
Event is created by Graph (*:1)

### GuardRun
GuardRun has Name (*:1)
GuardRun is for Guard (*:1)
GuardRun references Graph (*:1)
