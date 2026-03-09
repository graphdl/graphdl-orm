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

Constraint Type(.code) is an entity type.

ConstraintSpan objectifies "Constraint spans Role".

Modality Type is a value type.

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

HTTP Operation(.code) is an entity type.
  The possible values of HTTP Operation are 'POST', 'GET', 'PUT', 'PATCH', 'DELETE'.

ReadingIsUsedByVerb objectifies "Reading is used by Verb".

API objectifies "ReadingIsUsedByVerb is by HTTP Operation".

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

Name is a value type.
Text is a value type.
URI is a value type.
Timestamp is a value type.
Argument Length is a value type.

## Fact Types

### Noun
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

### Role
Constraint spans Role.
Role is used in Reading.

### Verb
Verb has Name.
Reading is used by Verb.
Event Type can be created by Verb.
Graph is referenced by Verb.
Verb is performed during Transition (Mealy semantics).
Verb is performed in Status (Moore semantics).

### Constraint
Constraint is of Constraint Type.
Constraint has modality of Modality Type.
Constraint spans Role.

### Set Comparison Constraint (subtype of Constraint)
Set Comparison Constraint has Argument Length.

### ConstraintSpan (objectification of "Constraint spans Role")
ConstraintSpan autofills from superset.

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

### API (objectification of "ReadingIsUsedByVerb is by HTTP Operation")
API has endpoint URI.

### ReadingIsUsedByVerb (objectification of "Reading is used by Verb")
ReadingIsUsedByVerb is by HTTP Operation.

### UI Element
Noun is displayed by UI Element.

### Toolbar
Toolbar has Toolbar Item.

### Menu
Menu has Menu Button.
