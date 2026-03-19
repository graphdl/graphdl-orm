# GraphDL Instances — Runtime Entities

## Entity Types

| Entity | Reference Scheme |
|--------|-----------------|
| Graph | (within Domain) |
| Resource | Reference (within Domain) |
| Resource Role | (within Graph) |
| State Machine | Name (within Domain) |
| Event | Timestamp (within State Machine) |
| Guard Run | Name (within Event) |
| Citation | (id) |

## Readings

### Citation
Citation has Text.
  Each Citation has exactly one Text.
Citation has URI.
  Each Citation has at most one URI.
Citation has Retrieval Date.
  Each Citation has at most one Retrieval Date.

### Graph
Graph belongs to Domain.
  Each Graph belongs to exactly one Domain.
Graph is of Graph Schema.
  Each Graph is of exactly one Graph Schema.
Graph cites Citation.
  For each pair of Graph and Citation, that Graph cites that Citation at most once.

### Resource
Resource belongs to Domain.
  Each Resource belongs to exactly one Domain.
Resource is instance of Noun.
  Each Resource is instance of exactly one Noun.
Resource has Reference.
  Each Resource has at most one Reference.
Resource has Value.
  Each Resource has at most one Value.

### Resource Role
Graph uses Resource for Role.
  Each Graph uses at most one Resource for each Role.

### State Machine
State Machine belongs to Domain.
  Each State Machine belongs to exactly one Domain.
State Machine has Name.
  Each State Machine has exactly one Name.
State Machine is instance of State Machine Definition.
  Each State Machine is instance of exactly one State Machine Definition.
State Machine is currently in Status.
  Each State Machine is currently in exactly one Status.
State Machine is for Resource.
  Each State Machine is for at most one Resource.

### Event
Event belongs to State Machine.
  Each Event belongs to exactly one State Machine.
Event is of Event Type.
  Each Event is of exactly one Event Type.
Event occurred at Timestamp.
  Each Event occurred at exactly one Timestamp.
Event is created by Graph.
  Each Event is created by at most one Graph.

### Guard Run
Guard Run has Name.
  Each Guard Run has at most one Name.
Guard Run is for Guard.
  Each Guard Run is for exactly one Guard.
Guard Run references Graph.
  Each Guard Run references at most one Graph.
