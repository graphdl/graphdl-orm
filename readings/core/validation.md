# AREST Validation: ORM2 Modeling Rules

## Deontic Constraints

### Noun Declaration

It is obligatory that each Role references exactly one Noun.

### Arity Decomposition

It is forbidden that a Constraint of Constraint Type 'UC' spans fewer Roles than the arity of its Fact Type minus one.

### Ring Constraint Completeness

It is obligatory that when a Fact Type has exactly two Roles that both reference the same Noun, some Constraint of Constraint Type 'IR', 'AS', 'AT', 'SY', 'IT', 'TR', or 'AC' spans those Roles.

It is permitted that a Fact Type has no Constraint of Constraint Type 'IR', 'AS', 'AT', 'SY', 'IT', 'TR', or 'AC' spanning its Roles when the Reading of that Fact Type contains a capitalized-word-prefixed form of its Ring Noun, or when some Noun ending in that Ring Noun is declared elsewhere in the corpus. The two conditions reflect compound-noun parse-time artifacts (eu-law `Personal Data Breach … Personal Data` and Biometric/Genetic/Personal Data sharing the `Data` suffix) and are read by `check_ring_completeness` to suppress ring-completeness hints; without them, the corpus surfaces 9 false-positive ring hints. The permission text is the source of truth for the suppression patterns — `check.rs` reads the Permission cell and applies the named pattern matchers; deleting either condition here re-enables the corresponding suppression layer to drop out of the check.

### Ring Constraint Validity

It is forbidden that a Constraint of Constraint Type 'IR', 'AS', 'AT', 'SY', 'IT', 'TR', or 'AC' spans Roles of a Fact Type where those Roles reference different Nouns.

### Singular Naming

It is forbidden that Noun has Name that ends in 's' when that Name is a plural form.

### Alethic Before Deontic

It is forbidden that a Constraint has Modality Type 'Deontic' when that Constraint could be enforced as Modality Type 'Alethic'.

### Derivation Over Storage

It is forbidden that a Role stores a value that is derivable from existing Fact instances and Constraint spans.

### Subtype Constraint Declaration

It is obligatory that each subtype Noun has some totality or exclusion Constraint declared for its supertype relationship.

### Reference Scheme Redundancy

It is forbidden that a Reading restates a Noun reference scheme as a separate fact type.

### Elementary Fact Decomposition

It is forbidden that a Reading conjoins two independent assertions using 'and' when they can be expressed as separate Readings.

### Derivation Rule Acyclicity

No Derivation Rule depends on itself.
If Derivation Rule 1 depends on Derivation Rule 2, then Derivation Rule 2 does not depend on Derivation Rule 1.

### Derivation Rule Range Restriction

It is obligatory that each variable in a Derivation Rule consequent appears in at least one antecedent of that Derivation Rule.

## Instance Facts

Domain 'validation' has Access 'public'.
Domain 'validation' has Description 'Deontic constraints encoding ORM 2 / FORML 2 modeling discipline at the framework level. Meta-constraints about how domain models should be structured. Every domain inherits them.'.
