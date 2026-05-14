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

## Constraint Violation Templates (#898)

Violation Template is a value type.

Constraint Kind has Violation Template.
  Each Constraint Kind has at most one Violation Template.

### Placeholders

Each template carries one or more `{name}` substitution markers. The
per-kind resolver in `compile.rs` maps each name to a `Vec<Func>`
that runtime-evaluates to the atoms inserted at that position:

- `{value}` — the offending role value (single-role kinds: IR, AC,
  RF), via `role_value(0)` or the role index of the constrained
  span's single role.
- `{x}`, `{y}`, `{z}` — role values for binary / ternary ring chains
  (AS, SY, AT, IT, TR). `{x}` is role 0 of the first fact, `{y}` is
  role 1 (and the second-fact role 0 for chains), `{z}` is role 1
  of the second fact.
- `{noun}` — the constrained noun's declared name, as a string atom.
- `{reading}` — the constrained fact type's reading string.
- `{range}` — the frequency constraint's `exactly N` / `between M
  and N` / `at least N` phrase. Resolver builds the phrase from the
  constraint's `min_occurrence` / `max_occurrence` at compile time.
- `{valid_set}` — value-constraint's allowed-value set, joined as
  `{A, B, C}` (curly braces in the substituted atom, not in the
  template).
- `{entity}`, `{requirement}`, `{clause_count}` — set-comparison
  values: the entity noun being scoped over, the kind's English
  requirement (`exactly one` / `at most one` / `at least one`), and
  the clause-fact-type count.
- `{a_ft}`, `{b_ft}` — subset / equality fact-type IDs (left and
  right side of the directional subset check).
- `{pairs}` — subset / equality multi-segment placeholder: expands
  to one `<noun, value>` pair per common-noun join column.

### Templates

Constraint Kind 'IR' has Violation Template 'Irreflexive violation: {value} references itself'.
Constraint Kind 'AS' has Violation Template 'Asymmetric violation: {x} relates to {y} and vice versa'.
Constraint Kind 'SY' has Violation Template 'Symmetric violation: {x} relates to {y} but not the reverse'.
Constraint Kind 'AT' has Violation Template 'Antisymmetric violation: {x} and {y} relate to each other but are not the same'.
Constraint Kind 'IT' has Violation Template 'Intransitive violation: {x} relates to {y} relates to {z} but shortcut also exists'.
Constraint Kind 'TR' has Violation Template 'Transitive violation: {x} relates to {y} relates to {z} but shortcut is missing'.
Constraint Kind 'AC' has Violation Template 'Acyclic violation: cycle detected through {value}'.
Constraint Kind 'RF' has Violation Template 'Reflexive violation: {value} does not reference itself'.
Constraint Kind 'UC' has Violation Template 'Uniqueness violation: {noun} {value} is not unique in {reading}'.
Constraint Kind 'MC' has Violation Template 'Mandatory violation: {noun} {value} does not participate in {reading}'.
Constraint Kind 'FC' has Violation Template 'Frequency violation: {noun} {value} in {reading} expected {range}'.
Constraint Kind 'VC' has Violation Template 'Value constraint violation: {noun} {value} is not in {valid_set}'.
Constraint Kind 'XO' has Violation Template 'Set-comparison violation: {entity} {value} expected {requirement} of {clause_count} clause fact types'.
Constraint Kind 'XC' has Violation Template 'Set-comparison violation: {entity} {value} expected {requirement} of {clause_count} clause fact types'.
Constraint Kind 'OR' has Violation Template 'Set-comparison violation: {entity} {value} expected {requirement} of {clause_count} clause fact types'.
Constraint Kind 'SS' has Violation Template 'Subset violation: {pairs} participates in {a_ft} but not in {b_ft}'.
Constraint Kind 'EQ' has Violation Template 'Equality violation: {pairs} in {a_ft} but not in {b_ft}'.

### Deontic-path templates

Constraint Kind 'DF_pop' has Violation Template 'Forbidden fact present in {primary_ft}'.
Constraint Kind 'DF_cwa' has Violation Template 'Response contains forbidden {noun} {value}'.
Constraint Kind 'DF_owa' has Violation Template 'Response may violate: {text}'.
Constraint Kind 'DO_pop' has Violation Template 'Obligation violated in {primary_ft}'.
Constraint Kind 'DO_obl' has Violation Template 'Response missing obligatory {noun}'.
Constraint Kind 'DO_sender' has Violation Template 'Response missing obligatory SenderIdentity'.

## Instance Facts

Domain 'validation' has Access 'public'.
Domain 'validation' has Description 'Deontic constraints encoding ORM 2 / FORML 2 modeling discipline at the framework level. Meta-constraints about how domain models should be structured. Every domain inherits them.'.
