# Structural Fixes: Unified Parser, Deployable Collections, Readings Extraction

Date: 2026-03-07

## Context

GraphDL is a meta-framework for Object-Role Modeling where natural language readings are the source code. The ORM Core diagram (design/Core.png) defines the canonical model: Nouns participate in Readings via Roles within GraphSchemas. Constraints (UC, MC) are placed on Roles via ConstraintSpans. There is no explicit "multiplicity" — just role constraints.

The design has existed since May 2023. The 24 meta-collections (22 original + Domains + Functions) have been stable since November 2023. The generation pipeline (OpenAPI, Payload, XState, Mermaid, iLayer) was built in an 8-day AI-assisted sprint (Feb 28 - Mar 7, 2026). During that sprint, the AI tools introduced rigid parsing scaffolding (markdown table parsers, `*:1` notation, `parseDomainMarkdown()`, `parseStateMachineMarkdown()`) that bypasses the system's actual design: semantic parsing of natural language into ORM facts.

This document describes three structural fixes to align the implementation with the original design.

## Problem

Three core issues block the system from operating as designed:

1. **Multiple rigid parsers instead of one semantic parser.** The seed pipeline expects specifically-formatted markdown tables with a `*:1` column. FORML2 expresses constraints verbally ("each X has at most one Y"), not symbolically. The claim extractor already does semantic fact extraction but isn't used for seeding. The seed parser and claim extractor are two implementations of the same operation.

2. **Generated Payload collections don't run.** The Generator produces Payload collection TypeScript as static files in its output. These files are never deployed. Instance data has no domain-level home — it either goes into the EAV graph structure (wrong for application-scale data) or doesn't exist.

3. **No way to extract readings back from the DB.** The DB is the source of truth, but changes made via admin UI, agent actions, or API calls can't be snapshotted to source control. There's no reverse path from DB state to readable, diffable, committable text.

## Design

### 1. Unified Semantic Parser

Replace the multiple parsers with a single semantic parsing pipeline that handles all natural language input.

**Input**: any text — FORML2 readings, prose, constraint statements, state machine descriptions, instance facts.

**Output**: decomposed ORM facts — Nouns, GraphSchemas, Roles, Constraints, ConstraintSpans, Statuses, Transitions, instance Graphs/Resources/ResourceRoles.

**Three extraction tiers** (same pipeline, escalating cost):

- **Tier 1 — FORML2 patterns** (deterministic): Input follows known verbal patterns. "Each Customer has at most one Name" parses directly to a UC on the Customer role. "It is obligatory that SupportResponse not contain ProhibitedPunctuation" parses to a deontic MC. No LLM needed.

- **Tier 2 — Compiled rule matching** (deterministic): Known patterns from existing fact types in the DB. The current `buildMatchers()` + `matchText()` in `src/extract/matcher.ts` already does this. Regex-compiled from constraint instances. Fast, no LLM cost.

- **Tier 3 — LLM extraction** (semantic): Freeform text that doesn't match known patterns. The current `extract-semantic.ts` in apis does this. Expensive, handles anything.

**Key insight**: seeding, validation, and claim extraction are the same operation — take text, produce structured facts. The seed endpoint, the `/extract` endpoint, and the `/check` endpoint should share the same parser. Validation is extraction pointed at outputs instead of inputs.

**What gets replaced**:
- `src/seed/parser.ts`: `parseDomainMarkdown()`, `parseStateMachineMarkdown()`, `parseFORML2()` — all replaced by the unified parser
- `src/seed/deontic.ts`: grouping logic absorbed into the parser
- The `*:1` notation as input format — constraints expressed verbally or derived from the semantic parse

**What stays**:
- `src/extract/matcher.ts`: `buildMatchers()` + `matchText()` — this IS tier 2, already correct
- `src/seed/handler.ts`: `ensureNoun()`, `ensureStatus()`, `ensureTransition()` etc. — idempotent DB writers stay, they just receive input from the unified parser instead of from the markdown parser
- The `afterChange` hook on Readings that tokenizes by noun names and creates Roles — this is semantic parsing, already in the right direction

### 2. Deployable Generated Collections

Generated Payload collections register in the running graphdl-orm instance with domain-scoped access control.

**Approach**: The Generator's Payload output produces CollectionConfig objects. A build step materializes these from the DB into `src/collections/generated/` as TypeScript files. `payload.config.ts` imports all generated collections from that directory. Production requires build + deploy. Dev mode hot-reloads when files change.

**Every generated collection gets**:
- `domainField` (relationship to Domains collection) — inherited from `src/collections/shared/domainScope.ts`
- `instanceReadAccess` / `instanceWriteAccess` — from `src/collections/shared/instanceAccess.ts`
- Data is private by default (tenant == user.email), shareable via domain visibility (public)

**Access control derived from readings**: The current generator emits `({ req: { user } }) => Boolean(user)` for all access. Instead, it should analyze which noun types appear as subjects in readings to determine which roles can perform which operations. "Admin redrafts SupportResponse" → the generated SupportResponse collection's update access checks for Admin role. "Customer submits SupportRequest" → the generated SupportRequest collection's create access allows Customer.

**BYOD (Bring Your Own Database)**: Optional. The Domains collection can have a `databaseUri` field for future use. Default: same MongoDB instance as graphdl-orm.

**Schema validation**: On each generation, diff the new collection configs against what's currently deployed. Report drift. When readings change, regenerate and the diff IS the migration plan.

**Build flow**:
1. Readings change (via seed, admin UI, agent, or API)
2. Generator triggers (manually or via hook)
3. Generator queries DB, produces CollectionConfig objects
4. Build script writes `.ts` files to `src/collections/generated/`
5. `payload.config.ts` auto-imports from generated directory
6. Build + deploy (production) or hot-reload (dev)

### 3. Readings Generator Output

A 6th generator output format that extracts the current DB state back to natural language readings.

**Input**: domain ID (which domain to extract)

**Output**: natural language text in FORML2-compatible format — readable, diffable, committable to source control.

**Sections extracted**:
- Entity types with reference schemes
- Value types with constraints (format, pattern, enum, min/max)
- Readings (fact types) with verbal constraint expressions
- Deontic constraints with instance facts
- State machine definitions with statuses, transitions, guards
- Instance facts (Graphs/Resources/ResourceRoles rendered as readings)

**Purpose**: Close the sync loop. DB is source of truth. This output snapshots it for version control. Diff between generated readings and committed readings = drift report.

**Workflow**:
1. Work happens in the DB (seed, admin UI, agent actions)
2. `generate readings` for domain → produces text
3. Commit to source control
4. Later: edit the text, re-seed → DB updates
5. `generate readings` again → new snapshot

## What This Does NOT Cover

- Performance optimization (query caching, materialized views)
- Hot-reload without build (Payload 3.x requires build-time collection registration)
- Derived fact engine (cost-per-call calculations)
- Composite/master-detail UI layouts
- Event sourcing / audit trail

## Success Criteria

1. A reading written in natural language can be seeded into the DB without requiring markdown table format or `*:1` notation
2. The generated Payload collections for a domain are running in graphdl-orm with working CRUD and domain-scoped access control
3. The readings generator output produces text that, when re-seeded, produces the same DB state (round-trip fidelity)
