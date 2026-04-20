# Design Principles

## Readings are the source of truth

FORML2 readings simultaneously define the schema, constraints, derivation rules, state machines, and API shape. There is no separate schema file, no migration, no ORM config. The reading IS the executable (Theorem 2).

## Facts all the way down

Every value in the API is derived from facts via rho-application (Theorem 5). There is no middleware, no informal layer, no procedural glue. Authorization is a derivation. Validation is a restriction. Navigation is a projection.

## Think in facts, not fields

A "field" is a value type playing a role in a binary fact type. Don't say "add a status field" -- say "Order has Status. Each Order has exactly one Status." The constraint is part of the declaration, not an afterthought.

## No over-modeling

If a concept is already captured by the formalism, don't add a separate entity for it. Events are facts. Guards are predicates over the population. Authorization is a derivation rule. Don't introduce phantom entities for things the algebra already handles.

## One function

SYSTEM is the only function (Eq. 9). REST endpoints are applications of SYSTEM with distinct inputs. Adding a new operation or entity type does not change SYSTEM -- new operations are registered in DEFS, new entity types are added to the population.

## Self-modification preserves all theorems

Ingesting new readings at runtime (Corollary 5) produces a new D where all five theorems still hold. No theorem depends on D being fixed at initialization. Migration is ingestion. Versioning is the event stream. Backward compatibility is preserved by construction.

## Cell isolation

For each cell in D, at most one application that writes to it may be in progress at any time (Definition 2). RMAP assigns each entity to its own cell. A single-threaded runtime satisfies this trivially. A distributed system requires one writer per cell.

## Provenance is a fact

Facts that originate outside the local rho algebra -- runtime-registered platform functions (httpFetch, send_email, ML scorers) and federated fetches under OWA -- carry a paired Citation record so the origin is queryable. Citation is itself a fact in P, produced by rho (cell_push), so Theorem 5 still holds: every value in the representation is a rho-application, including the origin trail. The Authority Type enum distinguishes kinds: 'Runtime-Function' and 'Federated-Fetch' for platform-layer origins; 'Constitutional', 'Statute', etc. for the original legal-research vocabulary. Domains can write obligations over provenance the same way they write any other deontic: "It is obligatory that each Fact of Fact Type 'ML Score' cites some Citation where that Citation has Authority Type 'Runtime-Function'."

## Platform binding, two writers

DEFS has two writers. Compile writes the domain layer from readings. The runtime writes the platform layer via ↓DEFS (ast::register_runtime_fn in Rust, the equivalent surface in each host). Apply dispatches through DEFS uniformly; the engine does not privilege either writer. The runtime_registered_names cell records which names entered via the runtime, and that is the origin marker Citation provenance reads -- it is not a type-level distinction on the Func AST, because FFP dispatches by name.
