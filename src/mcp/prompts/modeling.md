# Domain Modeling Guide

## Model First, Implement Second

Every task goes through these steps in order:

1. Check existing readings and business rules
2. Design or clarify the model (propose readings, not code)
3. Implement only after the model is settled

## Entity vs Value Types

- **Entity**: identified by a reference scheme. Customer(.Email), Order(.OrderId).
- **Value**: identified by its literal. A string, number, date.
- Entities become collections. Values become fields on entities.

## Reference Schemes

Every entity has one. Simple: `Customer(.Email)`. Compound: `Account(.Customer + .OAuthProvider)`. Reference scheme facts are implicit.

## Elementary Facts and Arity

- **Unary**: 1 role. "Customer is active."
- **Binary**: 2 roles. "Customer submits Support Request." (most common)
- **Ternary**: 3 roles. "Plan charges Price per Interval."
- A UC on an n-ary must span at least n-1 roles.

## Multiplicity

Express as FORML2 constraints, not shorthand:
- Many-to-one: "Each Support Request has at most one Priority."
- One-to-one: "Each Customer has at most one API Key. Each API Key belongs to at most one Customer."
- Mandatory: "Each Domain has exactly one Visibility."

## Subtype Constraints

- **Totality**: "Each Person is a Male or a Female."
- **Exclusion**: "No Person is both a Male and a Female."
- **Partition**: "Person is partitioned into Male, Female."

## Ring Constraints

Apply when both roles are the same type. Check: irreflexive? asymmetric? acyclic? transitive?

## Objectification

Only objectify fact types with a spanning UC. The spanning UC becomes the preferred reference scheme.

## Derivation Rules

Derived facts are computed, not stored: `Person has Full Name := Person has First Name + ' ' + Person has Last Name.`

## Subset Constraints

"If some A R1 some B then that A R2 that B." Antecedent is the subset, consequent is the superset. Autofill populates subset from superset.

## Red Flags

- Adding a field to 20+ tables by hand (model it as a relationship)
- Boolean flags for state (use state machines)
- Stored fields for derivable values (use derivation rules)
- "Implementation detail" that is actually a domain concept
