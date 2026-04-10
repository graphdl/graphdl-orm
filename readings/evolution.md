# Domain Evolution

## Description

How the metamodel modifies itself. A Domain Change proposes new metamodel entities (readings, nouns, constraints). The proposed changes are validated via CSDP before being applied. This domain is self-referential: it describes how new readings get added to GraphDL using GraphDL's own primitives.

The Curry-Howard correspondence applies: proposing a new fact type is proposing a theorem. CSDP validation is the proof check. Successful ingestion is the proof. The system can only evolve by proving something new.

## Entity Types

Domain Change(.Change Id) is an entity type.
Signal(.Signal Id) is an entity type.
Model Element(.id) is an entity type.

## Value Types

Change Id is a value type.
Signal Id is a value type.
Rationale is a value type.
Signal Source is a value type.
  The possible values of Signal Source are 'Constraint Violation', 'Human', 'Error Pattern', 'Feature Request', 'Support Request'.

## Readings

### Domain Change

Domain Change proposes Reading.
Domain Change proposes Noun.
Domain Change proposes Constraint.
Domain Change proposes Graph Schema.
Domain Change proposes Status.
Domain Change proposes Transition.
Domain Change proposes Event Type.

Domain Change has Rationale.
  Each Domain Change has exactly one Rationale.

Domain Change targets Domain.
  Each Domain Change targets exactly one Domain.

### Signal

Signal leads to Domain Change.
  Each Signal leads to at most one Domain Change.

Signal has Signal Source.
  Each Signal has exactly one Signal Source.

## Subtypes

Model Element is a supertype of Reading.
Model Element is a supertype of Noun.
Model Element is a supertype of Constraint.
Model Element is a supertype of Graph Schema.
Model Element is a supertype of Status.
Model Element is a supertype of Transition.
Model Element is a supertype of Event Type.

## Constraints

Each Domain Change proposes some Model Element.
It is possible that the same Domain Change proposes more than one Model Element.

It is obligatory that each Domain Change has exactly one Rationale.

It is forbidden that a Domain Change targeting Domain 'evolution' is applied without Signal Source 'Human'.
It is forbidden that a Domain Change targeting Domain 'organizations' is applied without Signal Source 'Human'.
It is forbidden that a Domain Change targeting Domain 'core' is applied without Signal Source 'Human'.

## Derivation Rules

Domain Change is valid iff all proposed Model Elements are parseable as FORML 2 and the proposed population is consistent with the existing population in the target Domain and the proposed Constraints are satisfiable with the existing Constraints in the target Domain.

## Instance Facts

State Machine Definition 'Domain Change' is for Noun 'Domain Change'.
Status 'Proposed' is initial in State Machine Definition 'Domain Change'.

Transition 'review' is defined in State Machine Definition 'Domain Change'.
Transition 'review' is from Status 'Proposed'.
Transition 'review' is to Status 'Under Review'.
Transition 'review' is triggered by Event Type 'review-requested'.

Transition 'approve-change' is defined in State Machine Definition 'Domain Change'.
Transition 'approve-change' is from Status 'Under Review'.
Transition 'approve-change' is to Status 'Approved'.
Transition 'approve-change' is triggered by Event Type 'approved'.

Transition 'reject' is defined in State Machine Definition 'Domain Change'.
Transition 'reject' is from Status 'Under Review'.
Transition 'reject' is to Status 'Rejected'.
Transition 'reject' is triggered by Event Type 'rejected'.

Transition 'revise' is defined in State Machine Definition 'Domain Change'.
Transition 'revise' is from Status 'Under Review'.
Transition 'revise' is to Status 'Proposed'.
Transition 'revise' is triggered by Event Type 'revision-requested'.

Transition 'apply' is defined in State Machine Definition 'Domain Change'.
Transition 'apply' is from Status 'Approved'.
Transition 'apply' is to Status 'Applied'.
Transition 'apply' is triggered by Event Type 'applied'.

Domain 'evolution' has Access 'public'.
