# GraphDL Overview

GraphDL is an AREST system. Readings are FORML2 declarations that compile to FFP objects. The system function resolves entities through ρ. REST endpoints are applications of the system function. Ingesting new readings is self-modification.

## Core Workflow

1. Check existing readings and business rules before writing code
2. Propose readings and constraints, not implementation
3. Implement only after the model is settled

## Key Concepts

- **Fact type**: a CONS construction with roles. Applied to object instances it produces a fact.
- **Constraint**: a Filter(predicate) over the population. Two families: cardinality (count in bounds) and membership (tuple exists/absent in target set).
- **Derivation rule**: a COMP composition that forward-chains to the least fixed point.
- **State machine**: a foldl over an event stream. Pure replay.
- **DEFS**: persistent named definitions. ρ resolves facts to functions by looking up DEFS.
- **Cell**: an entity's 3NF row. One writer at a time (Definition 2).

## Constraint Types

| Kind | Pattern | Family |
|------|---------|--------|
| UC | Each A has at most one B | Cardinality [0,1] |
| MC | Each A has some B | Cardinality [1,inf) |
| FC | Each A has at least k and at most m B | Cardinality [k,m] |
| UC+MC | Each A has exactly one B | Cardinality [1,1] |
| XC | No A R and R the same B | Cardinality [0,1] on participation |
| XO | Exactly one of the following holds | Cardinality [1,1] on participation |
| OR | Each A R some B or R some C | Cardinality [1,inf) on participation |
| IR | No A R itself | Membership (self-match) |
| AS | If A1 R A2 then impossible A2 R A1 | Membership (reverse present) |
| SY | If A1 R A2 then A2 R A1 | Membership (reverse absent) |
| IT | If A1 R A2 and A2 R A3 then impossible A1 R A3 | Membership (transitive present) |
| TR | If A1 R A2 and A2 R A3 then A1 R A3 | Membership (transitive absent) |
| SS | If some A R1 some B then that A R2 that B | Membership (cross-fact-type) |
| EQ | A R1 some B if and only if A R2 some C | Membership (bidirectional SS) |
| VC | The possible values of X are 'a', 'b', 'c' | Membership (value set) |

## FORML2 Document Structure

```
# DomainName
## Entity Types
## Subtypes
## Value Types
## Fact Types
## Constraints
## Mandatory Constraints
## Subset Constraints
## Equality Constraints
## Exclusion Constraints
## Deontic Constraints
## Derivation Rules
## Instance Facts
```

## Noun Naming

Use spaces, not PascalCase: "Support Request" not "SupportRequest". Preserve acronyms: API, VIN, HTTP. Always singular.
