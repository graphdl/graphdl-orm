# AREST Instances: Runtime Entities

## Entity Types

Resource(.Reference) is an entity type.
Fact is an entity type.
  Fact is a subtype of Resource.
Event(.id) is an entity type.
This association with Event, Transition, Resource provides the preferred identification scheme for Event Triggered Transition.
Guard Run(.Name) is an entity type.
Citation(.id) is an entity type.

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

### Fact
Fact belongs to Domain.
  Each Fact belongs to exactly one Domain.
Fact is of Fact Type.
  Each Fact is of exactly one Fact Type.
Fact is completed.
Fact is example.
Fact cites Citation.
  For each pair of Fact and Citation, that Fact cites that Citation at most once.

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
Fact uses Resource for Role.
  Each Fact uses at most one Resource for each Role.
  Each Fact uses some Resource for some Role.
This association with Fact, Resource, Role provides the preferred identification scheme for Resource Role.

### State (absorbed into Resource cell by RMAP)
Resource is currently in Status.
  Each Resource is currently in at most one Status.

### Event
Event(.id) is an entity type.
Event is for Resource.
  Each Event is for exactly one Resource.
Event is of Event Type.
  Each Event is of exactly one Event Type.
Event occurred at Timestamp.
  Each Event occurred at exactly one Timestamp.
Event has Data.
  Each Event has at most one Data.

### Event Triggered Transition (objectification of "Event triggered Transition for Resource")
Event triggered Transition for Resource.
  In each population of Event triggered Transition for Resource, each Event, Transition, Resource combination occurs at most once.

## Subset Constraints

If some Event triggered some Transition for some Resource then that Event is of some Event Type
  where that Transition is triggered by that Event Type.

### Guard Run
Guard Run is for Guard.
  Each Guard Run is for exactly one Guard.
Guard Run references Fact.
  It is possible that some Guard Run references more than one Fact and that some Fact is referenced by more than one Guard Run.
  For each combination of Guard Run and Fact, that Guard Run references that Fact at most once.
Guard Run has Result.
  Each Guard Run has at most one Result.

## Instance Facts

Domain 'instances' has Access 'public'.
