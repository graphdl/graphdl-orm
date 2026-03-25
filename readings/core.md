# GraphDL Core Metamodel
# Extracted from NORMA ORM2 model (design/html/)

## Entity Types

Function(.id) is an entity type.
Noun is a subtype of Function.
  Graph Schema is a subtype of Noun.
  Status is a subtype of Noun.
  {Graph Schema, Status} are mutually exclusive subtypes of Noun.

Reading(.id) is an entity type.

Role(.id) is an entity type.

Verb is a subtype of Function.

Constraint(.id) is an entity type.
  Set Comparison Constraint is a subtype of Constraint.
  Frequency Constraint is a subtype of Constraint.
  {Set Comparison Constraint, Frequency Constraint} are mutually exclusive subtypes of Constraint.

Constraint Type(.code) is an entity type.

This association with Constraint, Role provides the preferred identification scheme for Constraint Span.

Modality Type is a value type.
  The possible values of Modality Type are 'Alethic', 'Deontic'.

World Assumption is a value type.
  The possible values of World Assumption are 'closed', 'open'.

This association with Reading, Verb provides the preferred identification scheme for Reading Is Used By Verb.

This association with Reading Is Used By Verb, HTTP Method provides the preferred identification scheme for API.

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
    {Button, Checkbox, Date Picker, Image, Label, Password Box, Select List, Slider, Text Area, Text Box, Time Picker} are mutually exclusive subtypes of Control.
  Grid is a subtype of UI Element.
  Menu is a subtype of UI Element.
  Menu Button is a subtype of UI Element.
  Search Box is a subtype of UI Element.
  Toolbar is a subtype of UI Element.
  Toolbar Item is a subtype of UI Element.
    Toolbar Button is a subtype of Toolbar Item.
    Toolbar Separator is a subtype of Toolbar Item.
    {Toolbar Button, Toolbar Separator} are mutually exclusive subtypes of Toolbar Item.
  Alert is a subtype of UI Element.
  {Control, Grid, Menu, Menu Button, Search Box, Toolbar, Toolbar Item, Alert} are mutually exclusive subtypes of UI Element.

schema:Thing(.Name) is an entity type.

## Value Types

Arity is a value type.
Position is a value type.
Min Occurrence is a value type.
Max Occurrence is a value type.
Name is a value type.
Plural is a value type.
Object Type is a value type.
  The possible values of Object Type are 'entity', 'value'.
Value Type Name is a value type.
Format is a value type.
Enum Values is a value type.
Minimum is a value type.
Maximum is a value type.
Exclusive Minimum is a value type.
Exclusive Maximum is a value type.
Multiple Of is a value type.
Min Length is a value type.
Max Length is a value type.
Pattern is a value type.
Description is a value type.
Text is a value type.
URI is a value type.
Header is a value type.
Timestamp is a value type.
Argument Length is a value type.
Order is a value type.
Data is a value type.
Result is a value type.

Permission is a value type.
  The possible values of Permission are 'create', 'read', 'update', 'delete', 'list', 'versioned', 'login', 'rateLimit'.

Role Relationship is a value type.
  The possible values of Role Relationship are 'many-to-one', 'one-to-many', 'many-to-many', 'one-to-one'.

HTTP Method is a value type.
  The possible values of HTTP Method are 'GET', 'POST', 'PUT', 'PATCH', 'DELETE'.

Function Type is a value type.
  The possible values of Function Type are 'httpCallback', 'query', 'agentInvocation', 'transform'.

Scope is a value type.
  The possible values of Scope are 'organization', 'public'.

## Fact Types

### Noun
Noun has Object Type.
  Each Noun has exactly one Object Type.
Noun has Plural.
  Each Noun has at most one Plural.
Noun has Value Type Name.
  Each Noun has at most one Value Type Name.
Noun has Format.
  Each Noun has at most one Format.
Noun has Enum Values.
  Each Noun has at most one Enum Values.
Noun has Minimum.
  Each Noun has at most one Minimum.
Noun has Maximum.
  Each Noun has at most one Maximum.
Noun has Pattern.
  Each Noun has at most one Pattern.
Noun has Description.
  Each Noun has at most one Description.
Noun has Exclusive Minimum.
  Each Noun has at most one Exclusive Minimum.
Noun has Exclusive Maximum.
  Each Noun has at most one Exclusive Maximum.
Noun has Multiple Of.
  Each Noun has at most one Multiple Of.
Noun has Min Length.
  Each Noun has at most one Min Length.
Noun has Max Length.
  Each Noun has at most one Max Length.
Noun has Permission.
Noun has Reference Scheme Noun.
Noun is subtype of Noun.
Noun is described to AI by prompt Text.
Noun has World Assumption.
  Each Noun has exactly one World Assumption.
Noun is independent.
Noun is of schema:Thing.
  Each Noun is of at most one schema:Thing.
  It is possible that more than one Noun is of the same schema:Thing.
Noun plays Role.
  Each Noun plays some Role.
  For each Role, exactly one Noun plays that Role.
  It is possible that some Noun plays more than one Role.

### Reading
Reading has Text.
  Each Reading has exactly one Text.
  It is possible that more than one Reading has the same Text.
Reading is used by Verb.
  Each Reading is used by exactly one Verb.
  It is possible that some Verb uses more than one Reading.
Reading is localized for Language.
  Each Reading is localized for at most one Language.
  It is possible that more than one Reading is localized for the same Language.
Reading is primary.
Role is used in Reading.
  Each Role is used in some Reading.
  For each Reading, some Role is used in that Reading.

### Graph Schema (subtype of Noun)
Graph Schema has Title.
  Each Graph Schema has at most one Title.
Graph Schema has Reading.
  Each Graph Schema has some Reading.
  For each Reading, exactly one Graph Schema has that Reading.
  It is possible that some Graph Schema has more than one Reading.
Graph Schema has Role.
  Each Graph Schema has some Role.
  For each Role, exactly one Graph Schema has that Role.
  It is possible that some Graph Schema has more than one Role.
Graph Schema has Arity.
Graph Schema has Order.
  Each Graph Schema has at most one Order.
Graph Schema has Role Relationship.
  Each Graph Schema has at most one Role Relationship.
Graph Schema is derived.

### Role
Constraint spans Role.
  Each Constraint spans some Role.
Role is used in Reading.
Role has Position for Reading.
  For each Role and Reading that Role has that Reading at most one Position.

### Verb
Verb has Name.
  Each Verb has exactly one Name.
  It is possible that more than one Verb has the same Name.
Reading is used by Verb.
Verb executes Function.
  Each Verb executes at most one Function.
Event Type can be created by Verb.
Graph is referenced by Verb.
  Each Graph is referenced by exactly one Verb.
  It is possible that some Verb references more than one Graph.
Verb is performed during Transition (Mealy semantics).
  For each Transition, at most one Verb is performed during that Transition.
  It is possible that some Verb is performed during more than one Transition.
Verb is performed in Status (Moore semantics).
  For each Status, at most one Verb is performed in that Status.
  It is possible that some Verb is performed in more than one Status.

### Function
Function has Name.
  Each Function has at most one Name.
Function has Function Type.
  Each Function has at most one Function Type.
Function has callback URI.
  Each Function has at most one callback URI.
Function has HTTP Method.
  Each Function has at most one HTTP Method.
Function has Header.
  Each Function has each Header at most once.
Function has Scope.
  Each Function has at most one Scope.

### Constraint
Constraint is of Constraint Type.
Constraint has modality of Modality Type.
Constraint has Text.
  Each Constraint has at most one Text.
Constraint is semantic.
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

### Event Type
Event Type has Name.
  Each Event Type has exactly one Name.
  It is possible that more than one Event Type has the same Name.
Event Type publishes to Stream.
  For each Stream, exactly one Event Type publishes to that Stream.
  It is possible that some Event Type publishes to more than one Stream.
Event Type can be created by Verb.

### Stream
Stream has Name.
  Each Stream has exactly one Name.
  It is possible that more than one Stream has the same Name.

### API (objectification of "Reading Is Used By Verb is by HTTP Method")
API has endpoint URI.
  Each API has exactly one endpoint URI.
  For each endpoint URI, at most one API has that endpoint URI.

### Reading Is Used By Verb (objectification of "Reading is used by Verb")
Reading Is Used By Verb is by HTTP Method.

### UI Element
Noun is displayed by UI Element.
  Each Noun is displayed by at most one UI Element.
  It is possible that more than one Noun is displayed by the same UI Element.

### Toolbar
Toolbar has Toolbar Item.

### Menu
Menu has Menu Button.

## Constraints

Each Constraint is of exactly one Constraint Type.
It is possible that more than one Constraint is of the same Constraint Type.

Each Constraint has modality of exactly one Modality Type.
It is possible that more than one Constraint has modality of the same Modality Type.

## Disjunctive Mandatory Constraints

For each Status, some Transition is from that Status or some Transition is to that Status.


## Subset Constraints

If some Role is used in some Reading where some Graph Schema has that Reading then that Graph Schema has that Role.
If some Graph uses some Resource for some Role then that Graph is of some Graph Schema that has that Role.
If some Graph uses some Resource for some Role then that Resource is instance of some Noun that plays that Role.
If some Graph Schema defines some Graph then some Resource that is that Graph is instance of some Noun that is that Graph Schema.
If some Verb references some Graph that is of some Graph Schema then that Verb uses some Reading where that Graph Schema has that Reading.
If some Guard Run is for some Guard and that Guard Run references some Graph then that Guard references some Graph Schema that defines that Graph.
If some State Machine is currently in some Status then that Status belongs to some State Machine Definition where that State Machine is instance of that State Machine Definition.

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
