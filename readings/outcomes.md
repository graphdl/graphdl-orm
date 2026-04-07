# GraphDL Outcomes — Violations and Failures as Facts

Violations and failures are first-class domain entities, not out-of-band error responses.
Every evaluation path returns either valid claims, violation facts, failure facts, or a combination. No silent paths.

## Entity Types

Violation(.id) is an entity type.
Failure(.id) is an entity type.
Batch(.id) is an entity type.

## Value Types

Failure Type is a value type.
  The possible values of Failure Type are 'extraction', 'evaluation', 'transition', 'parse', 'induction'.
Severity is a value type.
  The possible values of Severity are 'error', 'warning', 'info'.
Confidence is a value type.

## Fact Types

### Violation
Violation belongs to Domain.
  Each Violation belongs to exactly one Domain.
Violation is of Constraint.
  Each Violation is of exactly one Constraint.
Violation is against Function.
  Each Violation is against at most one Function.
Violation has Text.
  Each Violation has exactly one Text.
Violation has Severity.
  Each Violation has exactly one Severity.
Violation occurred at Timestamp.
  Each Violation occurred at exactly one Timestamp.
Violation belongs to Batch.
  Each Violation belongs to at most one Batch.

### Failure
Failure belongs to Domain.
  Each Failure belongs to at most one Domain.
Failure has Failure Type.
  Each Failure has exactly one Failure Type.
Failure is against Function.
  Each Failure is against at most one Function.
Failure has input Text.
  Each Failure has at most one input Text.
Failure has reason Text.
  Each Failure has exactly one reason Text.
Failure has Severity.
  Each Failure has exactly one Severity.
Failure occurred at Timestamp.
  Each Failure occurred at exactly one Timestamp.

### Causal Links
Failure is caused by Violation.
  Each Failure is caused by at most one Violation.
Violation is triggered by Resource.
  Each Violation is triggered by at most one Resource.
Failure occurs during Transition.
  Each Failure occurs during at most one Transition.

### Temporal Ordering
Failure follows Violation.
  Each Failure follows at most one Violation.
Violation occurs before Transition.
  Each Violation occurs before at most one Transition.

## Constraints

Each Violation is of exactly one Constraint.
Each Failure has exactly one Failure Type.

## Subset Constraints

If some Failure follows some Violation then that Failure is caused by that Violation or that Violation occurred at some Timestamp and that Failure occurred at some Timestamp where that Violation Timestamp is before that Failure Timestamp.
If some Violation occurs before some Transition then that Violation occurred at some Timestamp and that Transition occurred at some Timestamp where that Violation Timestamp is before that Transition Timestamp.

## Instance Facts

Domain 'outcomes' has Access 'public'.
