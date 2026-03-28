# GraphDL Validation — ORM2 Modeling Rules

Deontic constraints encoding ORM2/FORML2 modeling discipline at the framework level. These are meta-constraints: constraints about how domain models should be structured. Every domain inherits them.

## Deontic Constraints

### Noun Declaration

It is obligatory that each Role references exactly one Noun.

### Arity Decomposition

It is forbidden that a Constraint of Constraint Type 'UC' spans fewer Roles than the arity of its Graph Schema minus one.

### Ring Constraint Completeness

It is obligatory that when a Graph Schema has exactly two Roles that both reference the same Noun, some Constraint of Constraint Type 'IR', 'AS', 'AT', 'SY', 'IT', 'TR', or 'AC' spans those Roles.

### Ring Constraint Validity

It is forbidden that a Constraint of Constraint Type 'IR', 'AS', 'AT', 'SY', 'IT', 'TR', or 'AC' spans Roles of a Graph Schema where those Roles reference different Nouns.

### Singular Naming

It is forbidden that Noun has Name that ends in 's' when that Name is a plural form.

### Alethic Before Deontic

It is forbidden that a Constraint has Modality Type 'Deontic' when that Constraint could be enforced as Modality Type 'Alethic'.

### Derivation Over Storage

It is forbidden that a Role stores a value that is derivable from existing Graph instances and Constraint spans.

### Subtype Constraint Declaration

It is obligatory that when Noun is a subtype of another Noun, a totality or exclusion Constraint is declared for that subtype relationship.

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

Domain 'validation' has Visibility 'public'.
