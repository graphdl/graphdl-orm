# GraphDL Instances — Runtime Entities

## Entity Types

Resource(.Reference) is an entity type.
Graph is an entity type.
  Graph is a subtype of Resource.
Resource Role is an entity type.
State Machine(.Name) is an entity type.
Event(.id) is an entity type.
This association with Event, Transition, State Machine provides the preferred identification scheme for Event Triggered Transition In State Machine.
Guard Run(.Name) is an entity type.
Citation is an entity type.

## Value Types

Reference is a value type.
Value is a value type.
Retrieval Date is a value type.

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
Graph is completed.
Graph is example.
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
Resource is created by User.
  Each Resource is created by at most one User.

### Resource Role
Graph uses Resource for Role.
  Each Graph uses at most one Resource for each Role.
  Each Graph uses some Resource for some Role.

### State Machine
State Machine belongs to Domain.
  Each State Machine belongs to exactly one Domain.
State Machine is instance of State Machine Definition.
  Each State Machine is instance of exactly one State Machine Definition.
State Machine is currently in Status.
  Each State Machine is currently in exactly one Status.
State Machine is for Resource.
  Each State Machine is for exactly one Resource.
  For each Resource, at most one State Machine is for that Resource.

### Event
Event is of Event Type.
  Each Event is of exactly one Event Type.
Event occurred at Timestamp.
  Each Event occurred at exactly one Timestamp.
Event has Data.
  Each Event has at most one Data.

### Event Triggered Transition In State Machine (objectification of "Event triggered Transition in State Machine")
Event triggered Transition in State Machine.
  It is possible that for some Event and Transition, that Event triggered that Transition in more than one State Machine
    and that for some Event and State Machine, that Event triggered more than one Transition in that State Machine
    and that for some Transition and State Machine, more than one Event triggered that Transition in that State Machine.
  In each population of Event triggered Transition in State Machine, each Event, Transition, State Machine combination occurs at most once.

## Subset Constraints

If some Event triggered some Transition in some State Machine then that Event is of some Event Type
  where that Transition is triggered by that Event Type.

### Guard Run
Guard Run is for Guard.
  Each Guard Run is for exactly one Guard.
Guard Run references Graph.
  It is possible that some Guard Run references more than one Graph and that some Graph is referenced by more than one Guard Run.
  For each combination of Guard Run and Graph, that Guard Run references that Graph at most once.
Guard Run has Result.
  Each Guard Run has at most one Result.
