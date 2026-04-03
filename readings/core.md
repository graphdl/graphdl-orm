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
  HTTP Method is a subtype of Verb.

Constraint(.id) is an entity type.
  Set Comparison Constraint is a subtype of Constraint.
  Frequency Constraint is a subtype of Constraint.
  {Set Comparison Constraint, Frequency Constraint} are mutually exclusive subtypes of Constraint.

Constraint Type(.code) is an entity type.

Derivation Rule(.id) is an entity type.

This association with Constraint, Role provides the preferred identification scheme for Constraint Span.

Modality Type is a value type.
  The possible values of Modality Type are 'Alethic', 'Deontic'.

World Assumption is a value type.
  The possible values of World Assumption are 'closed', 'open'.

This association with Graph Schema, Verb provides the preferred identification scheme for API.


Language(.code) is an entity type.

schema:Thing(.Name) is an entity type.

External System(.Name) is an entity type.

## Value Types

URL is a value type.
Secret Reference is a value type.

Arity is a value type.
Position is a value type.
Min Occurrence is a value type.
Max Occurrence is a value type.
Name is a value type.
Plural is a value type.
Object Type is a value type.
  The possible values of Object Type are 'entity', 'value'.
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
Prefix is a value type.
Header is a value type.
Timestamp is a value type.
Argument Length is a value type.
Order is a value type.
Data is a value type.
Result is a value type.
Title is a value type.

Permission is a value type.
  The possible values of Permission are 'create', 'read', 'update', 'delete', 'list', 'versioned', 'login', 'rateLimit'.

Role Relationship is a value type.
  The possible values of Role Relationship are 'many-to-one', 'one-to-many', 'many-to-many', 'one-to-one'.


Scope is a value type.
  The possible values of Scope are 'organization', 'public'.

## Fact Types

### Noun
Noun has Object Type.
  Each Noun has exactly one Object Type.
Noun has Plural.
  Each Noun has at most one Plural.
Noun has value-type- Name.
  Each value-type- Name belongs to at most one Noun.
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
  It is possible that some Verb is used by more than one Reading.
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
Graph Schema is activated by Verb.
  In each population of Graph Schema is activated by Verb, each Graph Schema, Verb combination occurs at most once.
Graph is referenced by Verb.
  It is possible that some Verb references more than one Graph.
  It is possible that more than one Verb references the same Graph.
Verb is performed during Transition (Mealy semantics).
  For each Transition, at most one Verb is performed during that Transition.
  It is possible that some Verb is performed during more than one Transition.
Verb is performed in Status (Moore semantics).
  For each Status, at most one Verb is performed in that Status.
  It is possible that some Verb is performed in more than one Status.

### Function
Function has Name.
  Each Function has at most one Name.
Function has callback URI.
  Each Function has at most one callback URI.
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

### API (objectification of "Graph Schema is activated by Verb")
API accepts Noun as parameter.
  Each API, Noun combination occurs at most once in the population of API accepts Noun as parameter.

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
If some State Machine is currently in some Status then that Status is defined in some State Machine Definition where that State Machine is instance of that State Machine Definition.
If some API accepts some Noun as parameter and some other Noun is subtype of that Noun then that API accepts that subtype Noun as parameter.

## Ring Constraints

No Noun is subtype of itself.
If Noun1 is subtype of Noun2, then Noun2 is not subtype of Noun1.
If Noun1 is subtype of Noun2 and Noun2 is subtype of Noun3, then Noun1 is subtype of Noun3.

### External System
External System has URL.
  Each External System has exactly one URL.
External System has Header.
  Each External System has at most one Header.
External System has Prefix.
  Each External System has at most one Prefix.
Noun is backed by External System.
  Each Noun is backed by at most one External System.
Function is backed by External System.
  Each Function is backed by at most one External System.

Noun has URI.
  Each Noun has at most one URI.

### Domain Connection
Domain connects to External System with Secret Reference.
  Each Domain has at most one Secret Reference per External System.

### Derivation Rule

Derivation Rule(.id) is an entity type.
Derivation Rule has Text.
  Each Derivation Rule has exactly one Text.
Derivation Rule has antecedent Graph Schema.
Derivation Rule produces Graph Schema.
  Each Derivation Rule produces exactly one Graph Schema.
Derivation Rule depends on Derivation Rule
  := Derivation Rule has antecedent Graph Schema
     and some other Derivation Rule produces that Graph Schema.

## Derivation Rules

Graph Schema has Arity := count of Role where Graph Schema has Role.

Constraint is semantic iff Constraint has modality of Modality Type 'Deontic' and Constraint spans some Role and that Role is played by some Noun and no Resource is instance of that Noun.

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

### HTTP Methods

HTTP Method 'GET' has Name 'GET'.
HTTP Method 'POST' has Name 'POST'.
HTTP Method 'PUT' has Name 'PUT'.
HTTP Method 'PATCH' has Name 'PATCH'.
HTTP Method 'DELETE' has Name 'DELETE'.
HTTP Method 'HEAD' has Name 'HEAD'.
HTTP Method 'OPTIONS' has Name 'OPTIONS'.

### External Systems

External System 'auth.vin' has URL 'https://auth.vin'.
External System 'auth.vin' has Header 'Authorization'.
External System 'auth.vin' has Prefix 'users API-Key'.
External System 'auto.dev' has URL 'https://api.auto.dev'.
External System 'auto.dev' has Header 'X-API-Key'.

Domain 'core' has Visibility 'public'.
