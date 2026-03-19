# GraphDL Core Metamodel
# Extracted from NORMA ORM2 model (design/html/)

## Entity Types

Noun(.id) is an entity type.
  Graph Schema is a subtype of Noun.
  Status is a subtype of Noun.

Reading(.id) is an entity type.

Role(.id) is an entity type.

Verb(.id) is an entity type.

Constraint(.id) is an entity type.
  Set Comparison Constraint is a subtype of Constraint.
  Frequency Constraint is a subtype of Constraint.

Constraint Type(.code) is an entity type.

Constraint Span objectifies "Constraint spans Role".

Modality Type is a value type.
  The possible values of Modality Type are 'Alethic', 'Deontic'.

Resource(.id) is an entity type.
  Graph is a subtype of Resource.

Event(.id) is an entity type.

Event Type(.id) is an entity type.

Stream(.id) is an entity type.

State Machine Definition(.id) is an entity type.

State Machine(.id) is an entity type.

Transition(.id) is an entity type.

Guard(.id) is an entity type.

Guard Run(.id) is an entity type.

Function(.id) is an entity type.

Reading Is Used By Verb objectifies "Reading is used by Verb".

API objectifies "Reading Is Used By Verb is by HTTP Method".

Language(.code) is an entity type.

UI Element(.id) is an entity type.
  Control is a subtype of UI Element.
    Button is a subtype of Control.
    Checkbox is a subtype of Control.
    Date Picker is a subtype of Control.
    Image is a subtype of Control.
    Label is a subtype of Control.
    Password Box is a subtype of Control.
    Select List is a subtype of Control.
    Slider is a subtype of Control.
    Text Area is a subtype of Control.
    Text Box is a subtype of Control.
    Time Picker is a subtype of Control.
  Grid is a subtype of UI Element.
  Menu is a subtype of UI Element.
  Menu Button is a subtype of UI Element.
  Search Box is a subtype of UI Element.
  Toolbar is a subtype of UI Element.
  Toolbar Item is a subtype of UI Element.
    Toolbar Button is a subtype of Toolbar Item.
    Toolbar Separator is a subtype of Toolbar Item.
  Alert is a subtype of UI Element.

schema:Thing(.Name) is an entity type.

## Value Types

Arity is a value type.
Min Occurrence is a value type.
Max Occurrence is a value type.
Name is a value type.
Object Type is a value type.
  The possible values of Object Type are 'entity', 'value'.
Text is a value type.
URI is a value type.
Header is a value type.
Timestamp is a value type.
Argument Length is a value type.

HTTP Method is a value type.
  The possible values of HTTP Method are 'GET', 'POST', 'PUT', 'PATCH', 'DELETE'.

Function Type is a value type.
  The possible values of Function Type are 'httpCallback', 'query', 'agentInvocation', 'transform'.

## Fact Types

### Noun
Noun has Object Type.
  Each Noun has exactly one Object Type.
Noun is subtype of Noun.
Noun is described to AI by prompt Text.
Noun is displayed by UI Element.
Noun is of schema:Thing.
Noun plays Role.

### Reading
Reading has Text.
Reading is used by Verb.
Reading is localized for Language.
Role is used in Reading.

### Graph Schema (subtype of Noun)
Graph Schema has Reading.
Graph Schema has Role.
Graph Schema has Arity.

### Role
Constraint spans Role.
Role is used in Reading.

### Verb
Verb has Name.
Reading is used by Verb.
Verb executes Function.
  Each Verb executes at most one Function.
Event Type can be created by Verb.
Graph is referenced by Verb.
Verb is performed during Transition (Mealy semantics).
Verb is performed in Status (Moore semantics).

### Function
Function has Function Type.
  Each Function has at most one Function Type.
Function has callback URI.
  Each Function has at most one callback URI.
Function has HTTP Method.
  Each Function has at most one HTTP Method.
Function has Header.
  Each Function has each Header at most once.

### Constraint
Constraint is of Constraint Type.
Constraint has modality of Modality Type.
Constraint spans Role.

### Constraint Type
Constraint Type has Name.
  Each Constraint Type has exactly one Name.

### Set Comparison Constraint (subtype of Constraint)
Set Comparison Constraint has Argument Length.

### Frequency Constraint (subtype of Constraint)
Frequency Constraint has Min Occurrence.
  Each Frequency Constraint has exactly one Min Occurrence.
Frequency Constraint has Max Occurrence.
  Each Frequency Constraint has at most one Max Occurrence.

### Constraint Span (objectification of "Constraint spans Role")
Constraint Span autofills from superset.

### Resource
Resource is of Noun.

### Graph (subtype of Resource)
Graph is of Graph Schema.
Graph is referenced by Verb.
Graph uses Resource for Role.
Graph is done for now.

### Event
Event is of Event Type.
Event is created by Graph.
Event is created by State Machine.
Event occurred at Timestamp.

### Event Type
Event Type has Name.
Event Type publishes to Stream.
Event Type can be created by Verb.

### Stream
Stream has Name.

### State Machine Definition
State Machine Definition is for Noun.
Status is defined in State Machine Definition.

### State Machine
State Machine is instance of State Machine Definition.
State Machine is for Resource.
State Machine is currently in Status.

### Status (subtype of Noun)
Transition is from Status.
Transition is to Status.
Verb is performed in Status (Moore semantics).

### Transition
Transition is from Status.
Transition is to Status.
Transition is triggered by Event Type.
Guard guards Transition.
Verb is performed during Transition (Mealy semantics).

### Guard
Guard guards Transition.
Guard references Graph Schema.

### Guard Run
Guard Run is run by Guard.
Guard Run references Graph.

### API (objectification of "Reading Is Used By Verb is by HTTP Method")
API has endpoint URI.

### Reading Is Used By Verb (objectification of "Reading is used by Verb")
Reading Is Used By Verb is by HTTP Method.

### UI Element
Noun is displayed by UI Element.

### Toolbar
Toolbar has Toolbar Item.

### Menu
Menu has Menu Button.

## Ring Constraints

No Noun is subtype of itself.
If Noun1 is subtype of Noun2, then Noun2 is not subtype of Noun1.
If Noun1 is subtype of Noun2 and Noun2 is subtype of Noun3, then Noun1 is subtype of Noun3.

## Derivation Rules

Graph Schema has Arity := count of Role where Graph Schema has Role.

## Instance Facts

### Constraint Types

Constraint Type 'UC' has Name 'Uniqueness'.
Constraint Type 'MC' has Name 'Mandatory'.
Constraint Type 'FC' has Name 'Frequency'.
Constraint Type 'SS' has Name 'Subset'.
Constraint Type 'EQ' has Name 'Equality'.
Constraint Type 'XC' has Name 'Exclusion'.
Constraint Type 'OR' has Name 'InclusiveOr'.
Constraint Type 'XO' has Name 'ExclusiveOr'.
Constraint Type 'IR' has Name 'Irreflexive'.
Constraint Type 'AS' has Name 'Asymmetric'.
Constraint Type 'AT' has Name 'Antisymmetric'.
Constraint Type 'SY' has Name 'Symmetric'.
Constraint Type 'IT' has Name 'Intransitive'.
Constraint Type 'TR' has Name 'Transitive'.
Constraint Type 'AC' has Name 'Acyclic'.
Constraint Type 'VC' has Name 'ValueComparison'.
