# AREST Core Metamodel

## Entity Types

Function(.id) is an entity type.
Noun is a subtype of Function.
  Fact Type is a subtype of Noun.
  Status is a subtype of Noun.
  {Fact Type, Status} are mutually exclusive subtypes of Noun.

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

Modality Type is a value type.
  The possible values of Modality Type are 'Alethic', 'Deontic'.

World Assumption is a value type.
  The possible values of World Assumption are 'closed', 'open'.

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

Derivation Mode is a value type.
  The possible values of Derivation Mode are 'fully-derived', 'derived-and-stored', 'semi-derived'.

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

### Fact Type (subtype of Noun)
Fact Type has Title.
  Each Fact Type has at most one Title.
Fact Type has Reading.
  Each Fact Type has some Reading.
  For each Reading, exactly one Fact Type has that Reading.
  It is possible that some Fact Type has more than one Reading.
Fact Type has Role.
  Each Fact Type has some Role.
  For each Role, exactly one Fact Type has that Role.
  It is possible that some Fact Type has more than one Role.
Fact Type has Arity. *
  Each Fact Type has exactly one Arity.
Fact Type has Order.
  Each Fact Type has at most one Order.
Fact Type has Role Relationship.
  Each Fact Type has at most one Role Relationship.
Fact Type has Derivation Mode.
  Each Fact Type has at most one Derivation Mode.

### Role
Constraint spans Role.
  Each Constraint spans some Role.
  This association with Constraint, Role provides the preferred identification scheme for Constraint Span.
Role is used in Reading.
Role has Position for Reading.
  For each Role and Reading that Role has that Reading at most one Position.

### Verb
Verb has Name.
  Each Verb has exactly one Name.
  It is possible that more than one Verb has the same Name.
Fact Type is activated by Verb.
  In each population of Fact Type is activated by Verb, each Fact Type, Verb combination occurs at most once.
  This association with Fact Type, Verb provides the preferred identification scheme for API.
Fact is referenced by Verb.
  It is possible that some Verb references more than one Fact.
  It is possible that more than one Verb references the same Fact.
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

### API (objectification of "Fact Type is activated by Verb")
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

If some Role is used in some Reading where some Fact Type has that Reading then that Fact Type has that Role.
If some Fact uses some Resource for some Role then that Fact is of some Fact Type that has that Role.
If some Fact uses some Resource for some Role then that Resource is instance of some Noun that plays that Role.
If some Fact Type defines some Fact then some Resource that is that Fact is instance of some Noun that is that Fact Type.
If some Verb references some Fact that is of some Fact Type then that Verb uses some Reading where that Fact Type has that Reading.
If some Guard Run is for some Guard and that Guard Run references some Fact then that Guard references some Fact Type that defines that Fact.
If some State Machine is currently in some Status then that Status is defined in some State Machine Definition where that State Machine is instance of that State Machine Definition.
If some API accepts some Noun as parameter and some other Noun is subtype of that Noun then that API accepts that subtype Noun as parameter.

## Ring Constraints

No Noun is subtype of itself.
If Noun1 is subtype of Noun2, then Noun2 is not subtype of Noun1.
If Noun1 is subtype of Noun2 and Noun2 is subtype of Noun3, then Noun1 is subtype of Noun3.

No Derivation Rule depends on itself.
If Derivation Rule 1 depends on Derivation Rule 2 and Derivation Rule 2 depends on Derivation Rule 3, then Derivation Rule 1 does not depend on Derivation Rule 3.

### External System
External System has URL.
  Each External System has exactly one URL.
External System has Header.
  Each External System has at most one Header.
External System has Prefix.
  Each External System has at most one Prefix.
External System has Kind.
  Each External System has at most one Kind.
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
Derivation Rule has antecedent Fact Type.
Derivation Rule produces Fact Type.
  Each Derivation Rule produces exactly one Fact Type.
Derivation Rule depends on Derivation Rule. *

## Derivation Rules

* Fact Type has Arity iff Arity is the count of Role where Fact Type has Role.

* Derivation Rule depends on Derivation Rule iff Derivation Rule has antecedent Fact Type and some other Derivation Rule produces that Fact Type.

Constraint is semantic iff Constraint has modality of Modality Type 'Deontic' and Constraint spans some Role and that Role is played by some Noun and no Resource is instance of that Noun.

## NORMA Structural Decomposition (#279)

The concepts below mirror NORMA's `ORMCoreMetaModel.orm`
decomposition of derivation rule bodies. They are the FORML 2
surface that the meta-circular parser (#280) populates by
decomposing each user-authored rule into a `Join Path` +
`Role Sequence` + `Role Projection`, rather than classifying the
rule text with Rust heuristics.

Paper §4 Table 1 correspondence:
  Join Path       ↔ Composition (COMP)
  Role Sequence   ↔ Construction (CONS)
  Role Projection ↔ Selector
  Join Type       ↔ Condition (COND)

### Entity types

Join Path(.id) is an entity type.
Join(.id) is an entity type.
Role Sequence(.id) is an entity type.
Role Projection(.id) is an entity type.
Join Type(.Name) is an entity type.

### Value types

Clusivity is a value type.
  The possible values of Clusivity are 'inclusive', 'exclusive'.

Derivation Storage Type is a value type.
  The possible values of Derivation Storage Type are 'stored', 'derived', 'derived-and-stored'.

### Fact types

Derivation Rule has Join Path.
  Each Derivation Rule has at most one Join Path.

Join Path has Join.
  Each Join Path has some Join.
  For each Join, exactly one Join Path has that Join.

Join uses Fact Type.
  Each Join uses exactly one Fact Type.

Join has Join Type.
  Each Join has exactly one Join Type.

Join has Role Sequence.
  Each Join has some Role Sequence.

Role Sequence has Role at Position.
  For each Role Sequence and Position, at most one Role is at that Position in that Role Sequence.

Role Projection is from Role Sequence.
  Each Role Projection is from exactly one Role Sequence.

Role Projection produces Role.
  Each Role Projection produces exactly one Role.

Derivation Rule has Role Projection.
  Each Derivation Rule has some Role Projection.

Fact Type has Derivation Storage Type.
  Each Fact Type has at most one Derivation Storage Type.

## NORMA Value Domain (#279)

### Entity types

Bound(.id) is an entity type.
Value Range(.id) is an entity type.
Facet(.id) is an entity type.
Value(.id) is an entity type.
Unit(.Name) is an entity type.
Dimension(.Name) is an entity type.
Textual Constraint is a subtype of Constraint.

### Value types

Regex Pattern is a value type.
Lexical Value is a value type.
Alias is a value type.
Length is a value type.
Binary Precision is a value type.
Digit Count is a value type.

### Fact types

Value is of Noun.
  Each Value is of exactly one Noun.

Value has Lexical Value.
  Each Value has exactly one Lexical Value.

Value Range has lower Bound.
  Each Value Range has at most one lower Bound.

Value Range has upper Bound.
  Each Value Range has at most one upper Bound.

Bound has Value.
  Each Bound has exactly one Value.

Bound has Clusivity.
  Each Bound has exactly one Clusivity.

Noun has Value Range.
  It is possible that more than one Noun has the same Value Range.

Noun has Facet.
  It is possible that more than one Noun has the same Facet.

Facet has Length.
  Each Facet has at most one Length.

Facet has Binary Precision.
  Each Facet has at most one Binary Precision.

Facet has Digit Count.
  Each Facet has at most one Digit Count.

Facet has Regex Pattern.
  Each Facet has at most one Regex Pattern.

Unit has Dimension.
  Each Unit has exactly one Dimension.

Noun is measured in Unit.
  Each Noun is measured in at most one Unit.

Textual Constraint has Text.
  Each Textual Constraint has exactly one Text.

Noun has Alias.
  It is possible that more than one Noun has the same Alias.

Fact Type has Alias.
  It is possible that more than one Fact Type has the same Alias.

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

### Join Types (NORMA #279)

Join Type 'inner' has Name 'inner'.
Join Type 'outer' has Name 'outer'.
Join Type 'left-outer' has Name 'left-outer'.
Join Type 'right-outer' has Name 'right-outer'.
Join Type 'anti' has Name 'anti'.

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
External System 'stripe' has URL 'https://api.stripe.com/v1'.
External System 'stripe' has Header 'Authorization'.
External System 'stripe' has Prefix 'Bearer'.

Domain 'core' has Access 'public'.
Domain 'core' has Description 'Extracted from NORMA ORM2 model (design/html/). The canonical FORML 2 metamodel against which every user domain is a subtype binding.'.
