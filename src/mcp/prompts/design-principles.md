# Design Principles

## The paper is the spec

AREST.tex defines the system. Implementation follows the paper, not the other way around. If the code diverges from the paper, the code is wrong.

## Facts all the way down

Every value in the API is derived from facts via rho-application (Theorem 5). There is no middleware, no informal layer, no procedural glue. Authorization is a derivation. Validation is a restriction. Navigation is a projection.

## One function

SYSTEM is the only function (Eq. 9). REST endpoints are applications of SYSTEM with distinct inputs. Adding a new operation or entity type does not change SYSTEM -- new operations are registered in DEFS, new entity types are added to the population.

## Readings are the source of truth

FORML2 readings simultaneously define the schema, constraints, derivation rules, state machines, and API shape. There is no separate schema file, no migration, no ORM config. The reading IS the executable (Theorem 2).

## Think in facts, not fields

A "field" is a value type playing a role in a binary fact type. Don't say "add a status field" -- say "Order has Status. Each Order has exactly one Status." The constraint is part of the declaration, not an afterthought.

## Self-modification preserves all theorems

Ingesting new readings at runtime (Corollary 5) produces a new D where all five theorems still hold. No theorem depends on D being fixed at initialization. Migration is ingestion. Versioning is the event stream. Backward compatibility is preserved by construction.

## Cell isolation

For each cell in D, at most one application that writes to it may be in progress at any time (Definition 2). RMAP assigns each entity to its own cell. A single-threaded runtime satisfies this trivially. A distributed system requires one writer per cell.

## No over-modeling

If a concept is already captured by the formalism, don't add a separate entity for it. Events are facts. Guards are predicates over the population. Authorization is a derivation rule. Don't introduce phantom entities for things the algebra already handles.
